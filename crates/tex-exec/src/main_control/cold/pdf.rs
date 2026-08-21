//! Cold PDF, effect, shipout, and observation helpers.
//!
//! These routines run only after typed command scanning or at a publication
//! barrier; none owns command delivery.

use super::super::*;
use super::operation::*;
use super::support::*;

pub(in crate::main_control) fn write_text<G>(
    tokens: tex_state::TokenListId<G>,
    stores: &tex_state::CommandContext<'_, G>,
) -> String {
    let mut text = String::new();
    for &word in stores.token_list(tokens) {
        tex_state::token_show::append_token_string_text(stores, word.token(), &mut text);
    }
    let mut text = crate::diagnostics::print_text_with_newlinechar(stores, &text);
    text.push('\n');
    text
}

/// TeX's eight-bit extension payload convention, with UTF-8 retained for
/// extended host-profile characters exactly as the legacy byte boundary does.
pub(in crate::main_control) fn tex_byte_text(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if let Ok(byte) = u8::try_from(ch as u32) {
            bytes.push(byte);
        } else {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
    }
    bytes
}

pub(in crate::main_control) fn pdf_graphics_text(
    tokens: TracedTokenList,
    stores: &Universe,
) -> Vec<u8> {
    let mut text = String::new();
    for &token in stores.tokens(tokens.token_ref().id()).iter() {
        tex_state::token_show::append_token_string_text(stores, token, &mut text);
    }
    tex_byte_text(&text)
}

pub(in crate::main_control) fn pdf_navigation_identity(
    stores: &Universe,
    identifier: &tex_state::PdfActionIdentifier,
) -> tex_state::PdfDestinationIdentity {
    match identifier {
        tex_state::PdfActionIdentifier::Number(number) => {
            tex_state::PdfDestinationIdentity::Number(*number)
        }
        tex_state::PdfActionIdentifier::Name(tokens) => tex_state::PdfDestinationIdentity::Name(
            pdf_graphics_text(TracedTokenList::synthetic(*tokens), stores),
        ),
        tex_state::PdfActionIdentifier::Raw(tokens) => tex_state::PdfDestinationIdentity::Name(
            pdf_graphics_text(TracedTokenList::synthetic(*tokens), stores),
        ),
    }
}

fn node_pdf_navigation_identifier(
    stores: &Universe,
    identifier: tex_state::PdfActionIdentifier,
) -> tex_state::node::NodePdfActionIdentifier {
    match identifier {
        tex_state::PdfActionIdentifier::Name(tokens) => {
            tex_state::node::NodePdfActionIdentifier::Name(tex_state::node::NodeTokenList::new(
                stores.tokens(tokens.id()).to_vec(),
            ))
        }
        tex_state::PdfActionIdentifier::Number(number) => {
            tex_state::node::NodePdfActionIdentifier::Number(number)
        }
        tex_state::PdfActionIdentifier::Raw(tokens) => {
            tex_state::node::NodePdfActionIdentifier::Raw(tex_state::node::NodeTokenList::new(
                stores.tokens(tokens.id()).to_vec(),
            ))
        }
    }
}

pub(in crate::main_control) fn apply_pdf_navigation_request(
    request: PdfNavigationRequest,
    stores: &mut Universe,
    modes: &mut ModeNest,
    fuel: &mut tex_command::CommandFuel,
) -> Result<ReplayStep, ExecError> {
    match request {
        PdfNavigationRequest::Annotation(request) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfannot"));
            }
            match request {
                PdfAnnotationRequest::Reserve => {
                    stores
                        .reserve_pdf_annotation()
                        .map_err(|_| ExecError::PdfObjectCapacity)?;
                }
                PdfAnnotationRequest::Define {
                    use_object,
                    dimensions,
                    entries,
                } => {
                    let data = tex_state::PdfAnnotationData {
                        dimensions,
                        entries: entries.tokens.token_ref(),
                    };
                    let record = match use_object {
                        Some(object) => stores
                            .initialize_pdf_annotation(
                                u32::try_from(object)
                                    .map_err(|_| ExecError::PdfReferencedObjectNotFound)?,
                                data,
                            )
                            .map_err(|_| ExecError::PdfReferencedObjectNotFound)?,
                        None => stores
                            .create_pdf_annotation(data)
                            .map_err(|_| ExecError::PdfObjectCapacity)?,
                    };
                    crate::box_runtime::append_whatsit(
                        modes,
                        stores,
                        fuel,
                        Whatsit::PdfAnnotation {
                            object: record.object(),
                        },
                    )?;
                }
            }
        }
        PdfNavigationRequest::StartLink(PdfStartLinkRequest {
            dimensions,
            attributes,
            action,
        }) => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                return Err(ExecError::PdfLinkInVerticalMode("pdfstartlink"));
            }
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfstartlink"));
            }
            let record = stores
                .create_pdf_link(
                    dimensions,
                    attributes.map_or(TokenListId::EMPTY, |value| value.tokens.token_list()),
                    action.clone(),
                    stores.execution_group_depth(),
                )
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            reserve_navigation_action_targets(stores, &action)?;
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                fuel,
                Whatsit::PdfLinkStart {
                    object: record.object(),
                },
            )?;
        }
        PdfNavigationRequest::EndLink => {
            if matches!(
                modes.current_mode(),
                Mode::Vertical | Mode::InternalVertical
            ) {
                return Err(ExecError::PdfLinkInVerticalMode("pdfendlink"));
            }
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfendlink"));
            }
            let open = stores
                .end_pdf_link()
                .ok_or(ExecError::PdfEndLinkWithoutStart)?;
            if open.nesting_depth != stores.execution_group_depth() {
                stores.world_mut().write_text(PrintSink::TerminalAndLog, "\npdfTeX warning: \\pdfendlink ended up in different nesting level than \\pdfstartlink\n");
            }
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                fuel,
                Whatsit::PdfLinkEnd {
                    object: open.record.object(),
                },
            )?;
        }
        PdfNavigationRequest::Outline(PdfOutlineRequest {
            attributes,
            action,
            count,
            title,
        }) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfoutline"));
            }
            stores
                .create_pdf_outline(
                    attributes.map_or(TokenListId::EMPTY, |value| value.tokens.token_list()),
                    action.clone(),
                    count,
                    title.tokens.token_list(),
                )
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            reserve_navigation_action_targets(stores, &action)?;
        }
        PdfNavigationRequest::Destination(PdfDestinationRequest {
            structure,
            identifier,
            kind,
        }) => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfdest"));
            }
            let identity = pdf_navigation_identity(stores, &identifier);
            if stores
                .pdf_destination(&identity, structure.is_some())
                .is_some_and(tex_state::PdfDestinationRecord::defined)
            {
                warn_pdf_destination_duplicate(stores, &identity);
                return Ok(ReplayStep::Continue);
            }
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                fuel,
                Whatsit::PdfDestination(Box::new(tex_state::node::PdfDestinationNode {
                    identifier: node_pdf_navigation_identifier(stores, identifier),
                    structure,
                    kind,
                })),
            )?;
        }
        PdfNavigationRequest::Thread(tex_command::PdfThreadRequest {
            dimensions,
            attributes,
            identifier,
            running,
        }) => {
            let primitive = if running {
                "pdfstartthread"
            } else {
                "pdfthread"
            };
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode(primitive));
            }
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                fuel,
                Whatsit::PdfThread(Box::new(tex_state::node::PdfThreadNode {
                    identifier: node_pdf_navigation_identifier(stores, identifier),
                    dimensions,
                    attributes: attributes.map_or_else(
                        tex_state::node::NodeTokenList::default,
                        |value| {
                            tex_state::node::NodeTokenList::new(
                                stores.tokens(value.tokens.token_ref().id()).to_vec(),
                            )
                        },
                    ),
                    running,
                })),
            )?;
        }
        PdfNavigationRequest::EndThread => {
            if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfendthread"));
            }
            crate::box_runtime::append_whatsit(modes, stores, fuel, Whatsit::PdfEndThread)?;
        }
    }
    Ok(ReplayStep::Continue)
}

pub(in crate::main_control) fn reserve_navigation_action_targets(
    stores: &mut Universe,
    action: &tex_state::PdfActionSpec,
) -> Result<(), ExecError> {
    let (destination, structure, thread) = pdf_action_target_identities(stores, action);
    if let Some(identity) = thread {
        stores
            .reserve_pdf_thread(identity)
            .map_err(|_| ExecError::PdfObjectCapacity)?;
    }
    if let Some(identity) = destination {
        stores
            .reserve_pdf_destination(identity, false)
            .map_err(|_| ExecError::PdfObjectCapacity)?;
    }
    if let Some(identity) = structure {
        stores
            .reserve_pdf_destination(identity, true)
            .map_err(|_| ExecError::PdfObjectCapacity)?;
    }
    Ok(())
}

pub(in crate::main_control) fn pdf_action_target_identities(
    stores: &Universe,
    action: &tex_state::PdfActionSpec,
) -> (
    Option<tex_state::PdfDestinationIdentity>,
    Option<tex_state::PdfDestinationIdentity>,
    Option<tex_state::PdfDestinationIdentity>,
) {
    let destination = match action {
        tex_state::PdfActionSpec::GoTo(destination) if destination.file.is_none() => destination,
        tex_state::PdfActionSpec::Thread(thread) if thread.file.is_none() => {
            let identity = match &thread.target {
                tex_state::PdfActionTarget::Destination(identifier) => {
                    Some(pdf_navigation_identity(stores, identifier))
                }
                tex_state::PdfActionTarget::Page { .. } => None,
            };
            return (None, None, identity);
        }
        _ => return (None, None, None),
    };
    let target = match &destination.target {
        tex_state::PdfActionTarget::Destination(identifier) => {
            Some(pdf_navigation_identity(stores, identifier))
        }
        tex_state::PdfActionTarget::Page { .. } => None,
    };
    let structure = destination
        .structure
        .as_ref()
        .map(|identifier| pdf_navigation_identity(stores, identifier));
    (target, structure, None)
}

pub(in crate::main_control) fn apply_pdf_graphics_request(
    request: PdfGraphicsRequest,
    stores: &mut Universe,
    modes: &mut ModeNest,
    command: &CommandState,
) -> Result<ReplayStep, ExecError> {
    use PdfColorStackActionRequest as Action;

    if !matches!(request, PdfGraphicsRequest::SavePosition)
        && stores.int_param(IntParam::PDF_OUTPUT) <= 0
    {
        let primitive = match request {
            PdfGraphicsRequest::Literal { .. } => "pdfliteral",
            PdfGraphicsRequest::SetMatrix { .. } => "pdfsetmatrix",
            PdfGraphicsRequest::Save => "pdfsave",
            PdfGraphicsRequest::Restore => "pdfrestore",
            PdfGraphicsRequest::ColorStack { .. } => "pdfcolorstack",
            PdfGraphicsRequest::SavePosition => unreachable!(),
            PdfGraphicsRequest::SnapReferencePoint => "pdfsnaprefpoint",
            PdfGraphicsRequest::SnapY { .. } => "pdfsnapy",
            PdfGraphicsRequest::SnapYComp { .. } => "pdfsnapycomp",
        };
        return Err(ExecError::PdfExtensionInDviMode(primitive));
    }

    let node = match request {
        PdfGraphicsRequest::Literal {
            mode,
            deferred: true,
            text,
        } => Node::Whatsit(Whatsit::DeferredPdfLiteral {
            mode,
            tokens: tex_state::node::NodeTokenList::new(
                stores.tokens(text.tokens.token_ref().id()).to_vec(),
            ),
        }),
        PdfGraphicsRequest::Literal { mode, text, .. } => Node::Whatsit(Whatsit::PdfLiteral {
            mode,
            payload: pdf_graphics_text(text.tokens, stores),
        }),
        PdfGraphicsRequest::SetMatrix { text } => Node::Whatsit(Whatsit::PdfSetMatrix {
            payload: pdf_graphics_text(text.tokens, stores),
        }),
        PdfGraphicsRequest::Save => Node::Whatsit(Whatsit::PdfSave),
        PdfGraphicsRequest::Restore => Node::Whatsit(Whatsit::PdfRestore),
        PdfGraphicsRequest::SavePosition => Node::Whatsit(Whatsit::PdfSavePos),
        PdfGraphicsRequest::SnapReferencePoint => Node::Whatsit(Whatsit::PdfSnapRefPoint),
        PdfGraphicsRequest::SnapY { glue } => {
            if glue.width.raw() < 0 {
                return Err(ExecError::PdfNavigation(
                    "pdfTeX error (ext1): negative snap glue",
                ));
            }
            Node::Whatsit(Whatsit::PdfSnapY { glue })
        }
        PdfGraphicsRequest::SnapYComp { ratio } => Node::Whatsit(Whatsit::PdfSnapYComp { ratio }),
        PdfGraphicsRequest::ColorStack { id, action } => {
            // pdftex.web's `<Implement \pdfcolorstack>` reports all three of
            // these through `print_err`/`error`, so each is a counted error
            // with a context display, not a bare note.
            let id = if id < 0 {
                let context = command.output_open_context(&stores.command_context());
                crate::error_report::report_error(
                    stores,
                    "Invalid negative color stack number",
                    &[
                        "I'll use default color stack 0 here.",
                        "Proceed, with fingers crossed.",
                    ],
                    context,
                )?;
                0
            } else if !stores.has_pdf_color_stack(id as u32) {
                let context = command.output_open_context(&stores.command_context());
                crate::error_report::report_error(
                    stores,
                    &format!("Unknown color stack number {id}"),
                    &[
                        "Allocate and initialize a color stack with \\pdfcolorstackinit.",
                        "I'll use default color stack 0 here.",
                        "Proceed, with fingers crossed.",
                    ],
                    context,
                )?;
                0
            } else {
                id as u32
            };
            let Some(action) = action else {
                let context = command.output_open_context(&stores.command_context());
                crate::error_report::report_error(
                    stores,
                    "Color stack action is missing",
                    &[
                        "The expected actions for \\pdfcolorstack:",
                        "    set, push, pop, current",
                        "I'll ignore the color stack command.",
                        "Proceed, with fingers crossed.",
                    ],
                    context,
                )?;
                return Ok(ReplayStep::Continue);
            };
            let action = match action {
                Action::Set(text) => {
                    tex_state::PdfColorStackAction::Set(pdf_graphics_text(text.tokens, stores))
                }
                Action::Push(text) => {
                    tex_state::PdfColorStackAction::Push(pdf_graphics_text(text.tokens, stores))
                }
                Action::Pop => tex_state::PdfColorStackAction::Pop,
                Action::Current => tex_state::PdfColorStackAction::Current,
            };
            Node::Whatsit(Whatsit::PdfColorStack { id, action })
        }
    };
    modes.current_list_mutation().push(node);
    Ok(ReplayStep::Continue)
}

pub(in crate::main_control) fn apply_pdf_object_request(
    request: PdfObjectRequest,
    stores: &mut Universe,
    immediate: bool,
) -> Result<ReplayStep, ExecError> {
    if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
        return Err(ExecError::PdfExtensionInDviMode("pdfobj"));
    }
    match request {
        PdfObjectRequest::Reserve => {
            stores
                .reserve_pdf_raw_object()
                .map_err(|_| ExecError::PdfObjectCapacity)?;
        }
        PdfObjectRequest::Define {
            use_object,
            stream,
            stream_attr,
            file,
            data,
        } => {
            let requested = use_object.and_then(|raw| {
                u32::try_from(raw).ok().and_then(|raw| {
                    stores
                        .pdf_raw_object(raw)
                        .filter(|record| record.data().is_none())
                        .map(|r| r.id())
                })
            });
            let id = match requested {
                Some(id) => id,
                None => {
                    if use_object.is_some() {
                        // pdftex.web §1542 publishes the sticky recovery
                        // sentinel before allocating the fallback object.
                        stores.set_pdf_return_value(-1);
                        stores.world_mut().write_text(
                            PrintSink::TerminalAndLog,
                            "\npdfTeX warning (\\pdfobj): invalid object number being ignored\n",
                        );
                    }
                    stores
                        .reserve_pdf_raw_object()
                        .map_err(|_| ExecError::PdfObjectCapacity)?
                }
            };
            stores
                .initialize_pdf_raw_object(
                    id,
                    stream,
                    stream_attr.map(|text| text.tokens.token_list()),
                    file,
                    data.tokens.token_list(),
                    immediate,
                )
                .map_err(|_| ExecError::PdfReferencedObjectNotFound)?;
        }
    }
    Ok(ReplayStep::Continue)
}

pub(in crate::main_control) fn apply_pdf_form_request(
    request: PdfFormRequest,
    stores: &mut Universe,
    modes: &mut ModeNest,
    command: &mut CommandMachine<'_>,
    immediate: bool,
) -> Result<ReplayStep, ExecError> {
    if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
        let name = match request {
            PdfFormRequest::Create { .. } => "pdfxform",
            PdfFormRequest::Reference { .. } => "pdfrefxform",
        };
        return Err(ExecError::PdfExtensionInDviMode(name));
    }
    match request {
        PdfFormRequest::Reference { object } => {
            let form = u32::try_from(object)
                .ok()
                .and_then(|object| stores.pdf_form(object))
                .ok_or(ExecError::PdfReferencedObjectNotFound)?;
            crate::box_runtime::append_whatsit(
                modes,
                stores,
                command.fuel,
                Whatsit::PdfRefXForm {
                    object: form.object(),
                    width: form.width(),
                    height: form.height(),
                    depth: form.depth(),
                },
            )?;
        }
        PdfFormRequest::Create {
            attr,
            resources,
            box_register,
        } => {
            // pdfTeX allocates the form identity before it consumes the box.
            let identity = stores
                .reserve_pdf_form()
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            let list = stores
                .take_box_to_page(box_register)
                .ok_or(ExecError::PdfXFormVoidBox)?;
            let dimensions = match stores
                .page_node_list(list)
                .expect("form source belongs to the live page arena")
                .get(0)
            {
                Some(Node::HList(node) | Node::VList(node)) => {
                    (node.width, node.height, node.depth)
                }
                _ => return Err(ExecError::PdfXFormVoidBox),
            };
            let form = stores
                .initialize_pdf_form(
                    identity,
                    list,
                    dimensions,
                    attr.map(|text| text.tokens.token_list()),
                    resources.map(|text| text.tokens.token_list()),
                    immediate,
                )
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            if immediate {
                // pdftex.web §1549's `do_extension` applies the immediate
                // prefix by traversing the captured form at creation time.
                // Use the same typed form traversal as lazy references so
                // graphics, saved positions, colors, and nested forms have
                // one ledger/artifact owner.
                let command = std::cell::RefCell::new(command);
                let mut write = |stores: &mut Universe, _: PrintSink, tokens: &[TokenWord]| {
                    replay_write(&mut command.borrow_mut(), stores, tokens, &mut Vec::new())
                };
                let mut replay = |stores: &mut Universe,
                                  kind: crate::shipout::ReplayTextKind,
                                  tokens: &[TokenWord]| {
                    replay_text(
                        &mut command.borrow_mut(),
                        stores,
                        kind,
                        tokens,
                        &mut Vec::new(),
                    )
                    .map(crate::shipout::ExpandedReplayText)
                };
                let artifact = crate::shipout::ShipoutTransaction::new(&mut write, &mut replay)
                    .stage_form(form.clone(), stores)?;
                stores.publish_pdf_traversal_positions(
                    artifact.last_position(),
                    artifact.snap_reference(),
                );
                stores.set_pdf_form_artifact(form.object(), artifact);
            }
        }
    }
    Ok(ReplayStep::Continue)
}

pub(in crate::main_control) fn replay_text<G>(
    command: &mut CommandMachine<'_, G>,
    stores: &mut LinearCommandContext<'_, G>,
    kind: crate::shipout::ReplayTextKind,
    tokens: &[TokenWord],
    diagnostics: &mut Vec<PendingDiagnostic>,
) -> Result<Vec<u8>, ExecError> {
    // Output replay is an isolated nested input transaction. Its synthetic
    // token levels may allocate immutable token lists and publish observer
    // events, but they must never advance or replace the surrounding source
    // cursor. This also gives a failing nested form replay an exact command
    // rollback boundary independent of the artifact/resource transaction.
    let input_snapshot = command.state.snapshot();
    let durable = stores
        .allocate_token_list(tokens)
        .expect("output replay fits admitted durable storage");
    let expanded = {
        let mut processor = command.processor(stores.take());
        let result = processor
            .expand_output_replay(durable)
            .map_err(command_error);
        diagnostics.extend(
            processor
                .take_semantic_diagnostics()
                .into_iter()
                .map(PendingDiagnostic::Command),
        );
        stores.restore(processor.into_context());
        result
    };
    command
        .state
        .rollback_nested_input_preserving_conditions(input_snapshot)
        .expect("shipout replay preserves the command profile");
    let expanded = expanded?;
    let mut text = String::new();
    let expanded = command
        .state
        .state()
        .attempt_token_words(expanded)
        .map_err(|_| ExecError::MissingToken {
            context: "expanded output replay",
        })?;
    for word in expanded {
        let token = word.token();
        match kind {
            crate::shipout::ReplayTextKind::Special => {
                tex_state::token_show::append_token_string_text(stores, token, &mut text);
            }
            crate::shipout::ReplayTextKind::PdfLiteral => {
                crate::diagnostics::append_token_show_text(stores, token, &mut text);
            }
        }
    }
    if matches!(kind, crate::shipout::ReplayTextKind::PdfLiteral) {
        return Ok(text.into_bytes());
    }
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if let Ok(byte) = u8::try_from(ch as u32) {
            bytes.push(byte);
        } else {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
    }
    Ok(bytes)
}

pub(in crate::main_control) fn replay_write<G>(
    command: &mut CommandMachine<'_, G>,
    stores: &mut LinearCommandContext<'_, G>,
    tokens: &[TokenWord],
    diagnostics: &mut Vec<PendingDiagnostic>,
) -> Result<crate::shipout::ExpandedWrite, ExecError> {
    let input_snapshot = command.state.snapshot();
    let durable = stores
        .allocate_token_list(tokens)
        .expect("write replay fits admitted durable storage");
    let expanded = {
        let mut processor = command.processor(stores.take());
        let result = processor
            .expand_durable_write_text(durable)
            .map_err(command_error);
        diagnostics.extend(
            processor
                .take_semantic_diagnostics()
                .into_iter()
                .map(PendingDiagnostic::Command),
        );
        stores.restore(processor.into_context());
        result
    };
    command
        .state
        .rollback_nested_input_preserving_conditions(input_snapshot)
        .expect("shipout write replay preserves the command profile");
    let expanded = expanded?;
    if expanded.unbalanced {
        crate::error_report::report_error(
            stores,
            "Unbalanced write command",
            &[
                "On this page there's a \\write with fewer real {'s than }'s.",
                "I can't handle that very well; good luck.",
            ],
            expanded
                .error_context
                .expect("unbalanced write retains its live input context"),
        )?;
    }
    let mut text = String::new();
    let words = command
        .state
        .state()
        .attempt_token_words(expanded.tokens)
        .map_err(|_| ExecError::MissingToken {
            context: "expanded write replay",
        })?;
    for word in words {
        tex_state::token_show::append_token_string_text(&**stores, word.token(), &mut text);
    }
    let mut text = crate::diagnostics::print_text_with_newlinechar(&**stores, &text);
    text.push('\n');
    Ok(crate::shipout::ExpandedWrite::transactional(text))
}

/// Selects TeX82 §1370 `write_out`'s destination for a stream number that
/// §1350's `new_write_whatsit` has already normalized into `0..=17`.
///
/// §1342: `write_open[17]` stands for every negative stream and
/// `write_open[16]` for every stream above 15, and both are permanently
/// closed. §1370 therefore sends 17 to the log alone (`if (j=17) and
/// (selector=term_and_log) then selector:=log_only`) and 16 to the terminal
/// and log.
pub(in crate::main_control) fn replay_write_sink(
    value: tex_command::WriteStreamSelector,
) -> PrintSink {
    match value {
        tex_command::WriteStreamSelector::Stream(slot) => PrintSink::Stream(StreamSlot::new(slot)),
        tex_command::WriteStreamSelector::Negative => PrintSink::Log,
        tex_command::WriteStreamSelector::AboveRange => PrintSink::TerminalAndLog,
    }
}

/// Selects TeX82 §1370's sink at the moment an immediate write executes.
///
/// An open numbered stream keeps its file selector. A closed numbered stream
/// and stream 16 keep the current interaction-mode selector; stream 17 only
/// redirects `term_and_log` to `log_only`.
pub(in crate::main_control) fn immediate_write_sink<G>(
    value: tex_command::WriteStreamSelector,
    stores: &tex_state::CommandContext<'_, G>,
) -> Option<PrintSink> {
    let interaction = match stores.interaction_mode_value() {
        0 => tex_state::InteractionMode::Batch,
        1 => tex_state::InteractionMode::Nonstop,
        2 => tex_state::InteractionMode::Scroll,
        _ => tex_state::InteractionMode::ErrorStop,
    };
    let selector = tex_state::print::Selector::for_interaction(interaction);
    match value {
        tex_command::WriteStreamSelector::Stream(slot) => {
            let slot = StreamSlot::new(slot);
            stores
                .output_stream_is_open(slot)
                .then_some(PrintSink::Stream(slot))
                .or_else(|| selector.sink())
        }
        tex_command::WriteStreamSelector::Negative
            if selector == tex_state::print::Selector::TermAndLog =>
        {
            Some(PrintSink::Log)
        }
        tex_command::WriteStreamSelector::Negative
        | tex_command::WriteStreamSelector::AboveRange => selector.sink(),
    }
}

/// Converts a stream number already normalized by its command-owned
/// restricted scan. Replay never owns range recovery or its diagnostic.
pub(in crate::main_control) fn replay_stream_slot(value: i32) -> StreamSlot {
    debug_assert!((0..tex_state::world::STREAM_SLOT_COUNT as i32).contains(&value));
    StreamSlot::new(value as u8)
}

pub(in crate::main_control) fn replay_openout_target(name: String) -> String {
    let mut path = PathBuf::from(name);
    if path.extension().is_none() {
        path.set_extension("tex");
    }
    path.to_string_lossy().into_owned()
}

// Mutation classification lives with each authoritative assignment commit.
/// Captures an executor-owned observable effect before application, then
/// emits it only after that application commits through the replay seam.
pub(in crate::main_control) fn pdf_image_dimensions(
    source: &tex_state::PdfExternalImageSource,
    width: Option<Scaled>,
    height: Option<Scaled>,
    depth: Option<Scaled>,
) -> tex_state::PdfExternalImageDimensions {
    let natural_width = source.natural_width;
    let natural_height = source.natural_height;
    let (width, height) = match (width, height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) if natural_width.raw() != 0 => (
            width,
            Scaled::from_raw(
                (i64::from(natural_height.raw()) * i64::from(width.raw())
                    / i64::from(natural_width.raw())) as i32,
            ),
        ),
        (None, Some(height)) if natural_height.raw() != 0 => (
            Scaled::from_raw(
                (i64::from(natural_width.raw()) * i64::from(height.raw())
                    / i64::from(natural_height.raw())) as i32,
            ),
            height,
        ),
        (Some(width), None) => (width, natural_height),
        (None, Some(height)) => (natural_width, height),
        (None, None) => (natural_width, natural_height),
    };
    tex_state::PdfExternalImageDimensions {
        width,
        height,
        depth: depth.unwrap_or_else(|| Scaled::from_raw(0)),
    }
}

/// Applies pdfTeX §§1550--1552's obsolete image-inclusion parameter aliases
/// before the effective request is exposed to the host. These writes remain
/// inside the step snapshot, so a resource suspension rolls them
/// back together with their diagnostics and retries the transition once.
pub(in crate::main_control) fn apply_pdf_image_compatibility_policy<G>(
    stores: &mut tex_state::CommandContext<'_, G>,
) {
    let obsolete_page_box = stores.int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX);
    if obsolete_page_box != 0 {
        stores.write_text(
            PrintSink::TerminalAndLog,
            "PDF inclusion: Primitive \\pdfoptionalwaysusepdfpagebox is obsolete; use \\pdfpagebox instead.\n",
        );
        stores
            .assign_int_param(
                IntParam::PDF_FORCE_PAGE_BOX,
                obsolete_page_box,
                tex_state::AssignmentScope::Global,
            )
            .expect("PDF parameter belongs to admitted state");
        stores
            .assign_int_param(
                IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX,
                0,
                tex_state::AssignmentScope::Global,
            )
            .expect("PDF parameter belongs to admitted state");
    }

    let obsolete_error_level = stores.int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL);
    if obsolete_error_level != 0 {
        stores.write_text(
            PrintSink::TerminalAndLog,
            "PDF inclusion: Primitive \\pdfoptionpdfinclusionerrorlevel is obsolete; use \\pdfinclusionerrorlevel instead.\n",
        );
        stores
            .assign_int_param(
                IntParam::PDF_INCLUSION_ERROR_LEVEL,
                obsolete_error_level,
                tex_state::AssignmentScope::Global,
            )
            .expect("PDF parameter belongs to admitted state");
        stores
            .assign_int_param(
                IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL,
                0,
                tex_state::AssignmentScope::Global,
            )
            .expect("PDF parameter belongs to admitted state");
    }
}

/// Applies pdfTeX's live `\pdfpagebox` and `\pdfforcepagebox` state after
/// command-owned source scanning but before the immutable host request is
/// exposed. This keeps `CommandProcessor` independent of `Universe` while
/// ensuring the host sees the effective page-box identity.
pub(in crate::main_control) fn pdf_image_page_box<G>(
    stores: &tex_state::CommandContext<'_, G>,
    request: &PdfImageRequest,
) -> tex_command::PdfImagePageBox {
    let page_box = |value| match value {
        1 => tex_command::PdfImagePageBox::Media,
        2 => tex_command::PdfImagePageBox::Crop,
        3 => tex_command::PdfImagePageBox::Bleed,
        4 => tex_command::PdfImagePageBox::Trim,
        5 => tex_command::PdfImagePageBox::Art,
        _ => tex_command::PdfImagePageBox::Crop,
    };
    let forced = stores.int_param(IntParam::PDF_FORCE_PAGE_BOX);
    if forced > 0 {
        page_box(forced)
    } else if request.page_box_explicit {
        request.page_box
    } else {
        page_box(stores.int_param(IntParam::PDF_PAGE_BOX))
    }
}

/// TeX82 §640's `dvi_out(eop); incr(total_pages)` is the one place a page
/// reaches the `.dvi` file, and §638's `ship_out` is the one routine that
/// reaches it.  The shipout effect therefore belongs to the page commit, not
/// to any command: §1075's `box_end` reaches `ship_out` for an explicit
/// `\shipout`, and §1012's `fire_up` reaches it again through §1025 for every
/// page the page builder ejects with a null `\output`.  Deriving the
/// observation from the committed-artifact delta covers both entry points --
/// and any later one -- by construction, so no command needs to know that it
/// happened to ship a page.
///
/// `total_pages` is incremented before the trace, so the published number is
/// the one-based ordinal of the page just committed.
pub(in crate::main_control) fn committed_shipout_observations<G>(
    before: usize,
    stores: &Universe<G>,
) -> Vec<EffectRecord> {
    (before..stores.world().artifact_commits().len())
        .map(|committed| EffectRecord {
            kind: ObservationEffectKind::Shipout,
            channel: "dvi".into(),
            value: ObservationValue::Integer(
                i64::try_from(committed.saturating_add(1))
                    .expect("the committed page count fits the oracle integer domain"),
            ),
            source: None,
        })
        .collect()
}

/// TeX82 §1374 performs open/close effects in `out_what`, whether §1375
/// reached it immediately or a whatsit reached it during later shipout.
/// Observe the committed `tex_state::EffectRecord` delta, not the command
/// spelling, so both entry paths publish the same ordered event exactly once.
pub(in crate::main_control) fn committed_stream_effect_observations<G>(
    before: usize,
    prepared_before: usize,
    stores: &Universe<G>,
    prepared_pages: &[crate::dispatch::PreparedDviPage],
) -> Vec<EffectRecord> {
    let shipped = &prepared_pages[prepared_before..];
    let direct = stores
        .world()
        .effect_records()
        .get(before..)
        .unwrap_or_default();
    if shipped.is_empty() {
        direct
            .iter()
            .filter_map(stream_effect_observation)
            .collect()
    } else {
        shipped
            .iter()
            .flat_map(|page| page.committed_effects.iter())
            .filter_map(stream_effect_observation)
            .collect()
    }
}

pub(in crate::main_control) fn stream_effect_observation(
    record: &tex_state::EffectRecord,
) -> Option<EffectRecord> {
    match record {
        tex_state::EffectRecord::StreamOpen { slot, target } => Some(EffectRecord {
            kind: ObservationEffectKind::Open,
            channel: format!("stream:{}", slot.raw()),
            value: ObservationValue::Name(target.path().to_string_lossy().into_owned()),
            source: None,
        }),
        tex_state::EffectRecord::StreamClose { slot } => Some(EffectRecord {
            kind: ObservationEffectKind::Close,
            channel: format!("stream:{}", slot.raw()),
            value: ObservationValue::None,
            source: None,
        }),
        _ => None,
    }
}

pub(in crate::main_control) fn write_effect_channel(sink: PrintSink) -> String {
    let stream = match sink {
        PrintSink::Stream(slot) => i32::from(slot.raw()),
        // TeX82 §§1342/1370 reserve selector 16 for writes above the stream
        // range and 17 for negative writes. `replay_write_sink` lowers those
        // selectors to their terminal/log routing before shipout.
        PrintSink::Terminal | PrintSink::TerminalAndLog => 16,
        PrintSink::Log => 17,
    };
    format!("stream:{stream}")
}

pub(in crate::main_control) fn applied_effect_observation<G>(
    scanned: &ColdOperation<G>,
    stores: &Universe<G>,
) -> Option<EffectRecord> {
    match scanned {
        ColdOperation::Message { tokens, .. } => Some(EffectRecord {
            kind: ObservationEffectKind::Message,
            // TeX82 §1279 observes the string produced by
            // `token_show(def_ref)`, not a character-only projection of the
            // expanded list. Control-sequence tokens can deliberately survive
            // expansion through `\noexpand` and must retain `print_cs`'s
            // spelling and separator.
            channel: "terminal".into(),
            value: ObservationValue::Bytes(
                message_tokens_text(stores, tokens.token_ref().id()).into_bytes(),
            ),
            source: None,
        }),
        ColdOperation::ShowTokens { tokens } => Some(EffectRecord {
            kind: ObservationEffectKind::ShowTokens,
            channel: "showtokens".into(),
            value: ObservationValue::Tokens(
                stores
                    .tokens(tokens.token_ref().id())
                    .iter()
                    .copied()
                    .map(|token| observed_macro_token(token, stores))
                    .collect(),
            ),
            source: None,
        }),
        ColdOperation::ShowIfs { conditions } => Some(EffectRecord {
            kind: ObservationEffectKind::ShowIfs,
            channel: "showifs".into(),
            value: ObservationValue::Name(render_showifs(conditions)),
            source: None,
        }),
        ColdOperation::ShowGroups {
            diagnostic: Some(diagnostic),
        } => Some(EffectRecord {
            kind: ObservationEffectKind::ShowGroups,
            channel: "showgroups".into(),
            value: ObservationValue::Name(crate::diagnostics::render_showgroups(diagnostic)),
            source: None,
        }),
        ColdOperation::ShowGroups { diagnostic: None } => None,
        ColdOperation::ImmediateExtension(ImmediateExtension::Write { stream, tokens }) => {
            Some(EffectRecord {
                kind: ObservationEffectKind::Write,
                channel: format!("stream:{}", stream.normalized_number()),
                value: ObservationValue::Tokens(
                    stores
                        .tokens(tokens.token_ref().id())
                        .iter()
                        .copied()
                        .map(|token| observed_macro_token(token, stores))
                        .collect(),
                ),
                source: None,
            })
        }
        // TeX82 §1335's `final_cleanup` and §1333's
        // `close_files_and_terminate` run once `its_all_over` has returned
        // true, so the job-termination effect belongs to that step and to no
        // other normal command.
        ColdOperation::End { .. } => Some(engine_termination_effect()),
        _ => None,
    }
}

pub(in crate::main_control) fn engine_termination_effect() -> EffectRecord {
    EffectRecord {
        kind: ObservationEffectKind::Terminate,
        channel: "engine".into(),
        value: ObservationValue::None,
        source: None,
    }
}

/// TeX82 §1075 completes `box_end` synchronously: a `\shipout` box is
/// published while its command-owned terminator backup is still live.  The
/// artifact kernel receives only an already-published detached input summary;
/// it never receives a legacy source stack or scans the command operand.
///
/// TeX82 §638's `ship_out` progress marker, opening half: the
/// `\tracingoutput` announcement, a leading separator, `[`, and the
/// nonzero-trimmed `\count0..\count9` values. Under `\tracingoutput>0` this
/// also closes the bracket and dumps the box, because §638 does:
///
/// ```text
/// if tracing_output>0 then
///   begin print_char("]"); begin_diagnostic; show_box(p); end_diagnostic(true);
///   end;
/// <Ship box p out>;
/// if eqtb[int_base+tracing_output_code].int<=0 then print_char("]");
/// ```
///
/// Everything here therefore precedes the page write, and
/// [`print_ship_out_marker_close`] follows it. tex.web's interleave is not
/// cosmetic: a `\write` whatsit inside the box prints *between* the two
/// halves, so `[7` opens the bracket, the write's text follows, and `]`
/// closes it.
pub(in crate::main_control) fn print_ship_out_marker_open<G>(
    stores: &mut Universe<G>,
    tracing_output: i32,
    counts: &[i32; 10],
    traced_node: Option<&Node>,
) -> usize {
    let last = (1..=9usize).rev().find(|&j| counts[j] != 0).unwrap_or(0);
    if tracing_output > 0 {
        let mut printer = stores.printer();
        printer.print_nl("");
        printer.print_ln();
        printer.print("Completed box being shipped out");
    }
    let marker_start;
    {
        let (term, log, max_print_line) = {
            let printer = stores.printer();
            (
                printer.terminal_offset(),
                printer.log_offset(),
                printer.max_print_line(),
            )
        };
        {
            let mut printer = stores.printer();
            if term > max_print_line.saturating_sub(9) {
                printer.print_ln();
            } else if term > 0 || log > 0 {
                printer.print_char(' ');
            }
        }
        marker_start = stores.world().effect_records().len();
        let mut printer = stores.printer();
        printer.print_char('[');
        for (index, &value) in counts.iter().enumerate().take(last + 1) {
            printer.print_int(value);
            if index < last {
                printer.print_char('.');
            }
        }
    }
    if let Some(node) = traced_node {
        stores.printer().print_char(']');
        let frozen = stores.publish_page_nodes(std::slice::from_ref(node));
        let text = crate::node_dump::dump_page_list(
            stores,
            frozen,
            crate::node_dump::DumpConfig::read(stores),
        );
        let mut diagnostic = stores.begin_diagnostic();
        // TeX82 §§174/198: `show_box` enters `show_node_list`, whose loop
        // executes `print_ln` before it renders the root node. This is an
        // unconditional structural break, not a `max_print_line` wrap and
        // not indentation carried by the detached node text.
        diagnostic.print_ln().print_rendered(&text);
        diagnostic.end(true);
    }
    marker_start
}

/// §638's `if eqtb[int_base+tracing_output_code].int<=0 then print_char("]")`,
/// run after the page has been written.
pub(in crate::main_control) fn print_ship_out_marker_close<G>(
    stores: &mut Universe<G>,
    tracing_output: i32,
) {
    if tracing_output <= 0 {
        stores.printer().print_char(']');
    }
}

/// TeX82 §638's allocator report after one page has been released.
///
/// The two live snapshots come from typed stores rather than the page
/// artifact, so enabling `\tracingstats` cannot perturb lowering or DVI
/// identity. `still untouched` is necessarily a compatibility projection:
/// Umber has no contiguous WEB memory gap, but retaining the profile's TeX
/// capacity makes the diagnostic monotone and keeps the allocator-specific
/// numbers isolated by the TRIP comparator's documented advisory policy.
pub(in crate::main_control) fn print_shipout_memory_usage<G>(
    stores: &mut Universe<G>,
    profile: CommandProfile,
    before: (usize, usize),
    after: (usize, usize),
) {
    // The typed arenas retain immutable history that WEB's mutable allocator
    // would have recycled. Bound each host-specific column to TeX's
    // three-digit diagnostic scale so that representation differences cannot
    // create an extra `max_print_line` break outside the advisory record.
    let project = |value: usize| value.min(999);
    let before = (project(before.0), project(before.1));
    let after = (project(after.0), project(after.1));
    let capacity = if profile == CommandProfile::ETEX26 {
        250_000usize
    } else {
        30_000usize
    };
    let untouched = capacity.saturating_sub(after.0.saturating_add(after.1));
    let mut printer = stores.printer();
    printer
        .print_nl("Memory usage before: ")
        .print_int(i64::try_from(before.0).unwrap_or(i64::MAX))
        .print_char('&')
        .print_int(i64::try_from(before.1).unwrap_or(i64::MAX))
        .print("; after: ")
        .print_int(i64::try_from(after.0).unwrap_or(i64::MAX))
        .print_char('&')
        .print_int(i64::try_from(after.1).unwrap_or(i64::MAX))
        .print("; still untouched: ")
        .print_int(i64::try_from(untouched).unwrap_or(i64::MAX))
        .print_ln();
}

pub(in crate::main_control) fn shipout_replay_box<G>(
    node: Node,
    stores: &mut Universe<G>,
    command: &mut CommandMachine<'_, G>,
) -> Result<Option<crate::dispatch::CommittedPagePublication>, ExecError> {
    // §638's `[` marker reports the page's `\count0`..`\count9` and, under
    // `\tracingoutput`, dumps the shipped box. Both are read before the page
    // is replayed, because replaying it is what changes them.
    let tracing_output = stores.int_param(IntParam::TRACING_OUTPUT);
    let tracing_stats = stores.int_param(IntParam::TRACING_STATS);
    let memory_before = (tracing_stats > 1).then(|| stores.shipout_memory_usage(Some(&node)));
    let counts: [i32; 10] =
        std::array::from_fn(|index| stores.count(u16::try_from(index).expect("0..=9 fits u16")));
    let traced_node = (tracing_output > 0).then(|| node.clone());
    let input_summary = stores.input_summary().clone();
    let output_open_context = command.state.output_open_context(&stores.command_context());
    // Effects live at this point are genuine whatsit output carried forward
    // from before the page; everything after it -- §638's own marker
    // included -- belongs to this shipout and must not be swept into the
    // page's serialized content.
    let pending_end = stores.world().effect_records().len();
    let marker_start =
        print_ship_out_marker_open(stores, tracing_output, &counts, traced_node.as_ref());
    let effect_start = stores.world().effect_records().len();
    let effect_cursor = std::cell::Cell::new(effect_start);
    let replay_diagnostics = std::cell::RefCell::new(Vec::new());
    let supports_pdftex_profile = command.state.profile().capabilities().supports_pdftex();
    let uses_pdftex_semantics = command.state.engine_semantics().supports_pdftex();
    let emit_dvi = command
        .emit_dvi_override
        .unwrap_or(!supports_pdftex_profile);
    let command_cell = std::cell::RefCell::new(command);
    let mut expand_write = |stores: &mut Universe<G>, sink: PrintSink, tokens: &[TokenWord]| {
        let mut command = command_cell.borrow_mut();
        let input_snapshot = command.state.snapshot();
        // TeX82 §§1374--1375 execute an open/close whatsit in `out_what`
        // before moving to the next whatsit. A following write expands only
        // after those effects have happened, so publish the committed prefix
        // before its nested command episode contributes observations.
        if let Some(observations) = command.observations.as_mut() {
            observations.extend(
                stores.world().effect_records()[effect_cursor.get()..]
                    .iter()
                    .filter_map(stream_effect_observation)
                    .map(CommandObservation::Effect),
            );
        }
        effect_cursor.set(stores.world().effect_records().len());
        let traced = tokens
            .iter()
            .copied()
            .map(|token| TracedTokenWord::pack(token, tex_state::token::OriginId::UNKNOWN))
            .collect::<Vec<_>>();
        let traced = stores.finish_traced_token_list(&traced);
        let expanded = {
            // TeX82 §1370 temporarily sets `mode:=0` while deferred
            // write text expands. §299 names that value "no mode", and
            // §367 updates `shown_mode` if it traces an expandable
            // command during the scan.
            let mode_prefix = command.shown_mode.is_some().then(|| "no mode".to_owned());
            let mut processor = command.processor(stores);
            processor.set_command_trace_mode_prefix(mode_prefix);
            let result = processor.expand_write_text(traced).map_err(command_error);
            let command_trace_printed = processor.command_trace_printed();
            let diagnostics = processor
                .take_semantic_diagnostics()
                .into_iter()
                .map(PendingDiagnostic::Command)
                .collect();
            drop(processor);
            // TeX82 §1370 performs expansion and then writes the
            // resulting token list on one live `write_out` call stack.
            // Publish §367 traces and scanner diagnostics into the
            // shipout transaction now, before normalization appends the
            // payload's stream effect.
            report_pending_diagnostics(stores, diagnostics)?;
            if command_trace_printed {
                *command.shown_mode = None;
            }
            result
        };
        command
            .state
            .rollback_nested_input_preserving_conditions(input_snapshot)
            .expect("shipout write replay preserves the command profile");
        let expanded = expanded?;
        if let Some(observations) = command.observations.as_mut() {
            observations.committed(CommandObservation::Effect(EffectRecord {
                kind: ObservationEffectKind::Write,
                channel: write_effect_channel(sink),
                value: ObservationValue::Tokens(
                    stores
                        .tokens(expanded.tokens.token_ref().id())
                        .iter()
                        .copied()
                        .map(|token| observed_macro_token(token, stores))
                        .collect(),
                ),
                source: None,
            }));
        }
        if expanded.unbalanced {
            // TeX82 §1372's `<Recover from an unbalanced write command>`.
            // Expansion diagnostics above, this report, and the recovered
            // payload all remain in their live-call order inside the
            // atomic page transaction.
            crate::error_report::report_error(
                stores,
                "Unbalanced write command",
                &[
                    "On this page there's a \\write with fewer real {'s than }'s.",
                    "I can't handle that very well; good luck.",
                ],
                expanded
                    .error_context
                    .expect("unbalanced write retains its live input context"),
            )?;
        }
        let mut text = String::new();
        for &token in stores.tokens(expanded.tokens.token_ref().id()).iter() {
            tex_state::token_show::append_token_string_text(stores, token, &mut text);
        }
        let mut text = crate::diagnostics::print_text_with_newlinechar(stores, &text);
        text.push('\n');
        Ok(crate::shipout::ExpandedWrite::transactional(text))
    };
    let mut expand_replay =
        |stores: &mut Universe<G>, kind: crate::shipout::ReplayTextKind, tokens: &[TokenWord]| {
            let mut command = command_cell.borrow_mut();
            let mut diagnostics = Vec::new();
            let result = replay_text(&mut command, stores, kind, tokens, &mut diagnostics)
                .map(crate::shipout::ExpandedReplayText);
            replay_diagnostics.borrow_mut().extend(diagnostics);
            result
        };
    let mut receipt =
        crate::shipout::ShipoutTransaction::new(&mut expand_write, &mut expand_replay).stage_page(
            node,
            input_summary,
            crate::shipout::ShipoutOrigin {
                output_open_context: Some(output_open_context),
                pending_end,
                // Web2C's `[53.1374]` notice belongs to the compiled engine,
                // not to the loaded format's command family. A pdfTeX binary
                // therefore retains it while executing a TeX82 profile.
                announce_openout: uses_pdftex_semantics,
            },
            stores,
            emit_dvi,
        )?;
    let command = command_cell.into_inner();
    if let Some(receipt) = receipt
        .as_mut()
        .and_then(|publication| publication.dvi.as_mut())
    {
        receipt.committed_effects = receipt.committed_effects[effect_cursor.get() - effect_start..]
            .to_vec()
            .into_boxed_slice();
    }
    // Deferred special/PDF-literal replay diagnostics remain command-owned
    // publications and cross the artifact transaction only after it commits.
    // Deferred writes publish their §1370 expansion diagnostics inside the
    // transaction so they precede the resulting stream payload.
    report_pending_diagnostics(stores, replay_diagnostics.into_inner())?;
    print_ship_out_marker_close(stores, tracing_output);
    if let Some(publication) = receipt.as_mut() {
        stores.world_mut().claim_effect_publication_boundary(
            pending_end..marker_start,
            marker_start,
            publication.artifact.effect(),
            publication
                .effect_output_attempt
                .expect("shipout assigns output-attempt ownership"),
        );
        publication.effects = marker_start..stores.world().effect_records().len();
        stores
            .world_mut()
            .claim_effect_publication(publication.effects.clone(), publication.artifact.effect());
    }
    if let Some(before) = memory_before {
        let after = stores.shipout_memory_usage(None);
        print_shipout_memory_usage(stores, command.state.profile(), before, after);
    }
    // The closing bracket prints after `shipout_node_with_input_summary`'s
    // own transaction has committed, so without this call it would sit as a
    // live, uncommitted effect suffix that a later `\shipout` would find at
    // the exact point `direct::stage_shipout` reads its carried-forward
    // effects. `pending_end` above already excludes this page's own marker
    // from that read; committing here keeps the *next* page's `pending_end`
    // from including this one's trailing `]` (`umber2-alfh.10`, confirmed
    // against
    // `effect_free_shipout_memo_republishes_one_aligned_receipt`'s
    // two identical `\shipout\copy0` calls). It is a no-op under retained
    // sessions, which consume their effect suffix on export instead.
    //
    // Memory-backed retained hosts checkpoint this materialized suffix at a
    // later resource suspension and reconcile its exact replay once.
    stores.commit_effects(stores.world().effect_pos())?;
    // TeX82's `ship_out` clears the consecutive-dead-output counter (§638).
    // The shipout boundary owns the page-state bookkeeping, so keep
    // the page-state transition at the typed shipout boundary.
    stores.set_page_integer(tex_state::page::PageInteger::DeadCycles, 0);
    Ok(receipt)
}

/// Renders a committed meaning the way the reference instrumentation's
/// `umber_trace_meaning_value` does.
///
/// tex.web stores a meaning as an `(eq_type, equiv)` pair and names it by its
/// command code, so the canonical rendering is a three-way split on the
/// command, never on how the meaning was reached:
///
/// - a macro (`eq_type >= call`) is its whole §294 body -- parameter text,
///   the `end_match` that separates the two halves, then replacement text;
/// - §208's `char_given` and `math_given` carry the shorthand code stored by
///   §1224 as a typed scalar;
/// - everything else is §207/§208's command name for the `eq_type`.
///
/// It must never fall back to a spelling (the source control sequence of a
/// `\let`) or to a Rust `Debug` rendering: both name where the meaning came
/// from rather than what it is (`umber2-johp.141`).
pub(in crate::main_control) fn meaning_mutation_value<G>(
    meaning: tex_state::meaning::ResolvedMeaning<G>,
    stores: &tex_state::CommandContext<'_, G>,
) -> ObservationValue {
    match meaning {
        tex_state::meaning::ResolvedMeaning::Macro { definition, flags } => {
            let macro_meaning = stores.definition(definition);
            ObservationValue::Tokens(observed_stored_macro_body(
                flags,
                macro_meaning.parameter_text(),
                macro_meaning.replacement_text(),
                stores,
            ))
        }
        tex_state::meaning::ResolvedMeaning::Static(Meaning::CharGiven(character)) => {
            ObservationValue::Character(u32::from(character))
        }
        tex_state::meaning::ResolvedMeaning::Static(Meaning::MathCharGiven(code)) => {
            ObservationValue::Integer(i64::from(code))
        }
        tex_state::meaning::ResolvedMeaning::Static(meaning) => {
            ObservationValue::Name(tex_command::canonical_names::meaning_command_name(meaning))
        }
    }
}

/// The macro body as stored by TeX82 §294 and e-TeX change section [49].
pub(in crate::main_control) fn observed_stored_macro_body<G>(
    flags: MeaningFlags,
    parameter_text: &[tex_state::token::TokenWord],
    replacement_text: &[tex_state::token::TokenWord],
    stores: &tex_state::CommandContext<'_, G>,
) -> Vec<ObservedToken> {
    let mut tokens = observed_macro_body(parameter_text, replacement_text, stores);
    if flags.contains(MeaningFlags::PROTECTED) {
        // e-TeX's `protected_token` is `other_token + "1"` where
        // `other_token` is command/category 14 (`comment`) times 256.
        tokens.insert(
            0,
            ObservedToken::Character {
                character: '\u{1}',
                catcode: tex_state::token::Catcode::Comment,
            },
        );
    }
    tokens
}

/// §294's stored macro body: parameter text, the separating `end_match`, then
/// replacement text, as one token sequence.
pub(in crate::main_control) fn observed_macro_body<G>(
    parameter_text: &[tex_state::token::TokenWord],
    replacement_text: &[tex_state::token::TokenWord],
    stores: &tex_state::CommandContext<'_, G>,
) -> Vec<ObservedToken> {
    let mut tokens = parameter_text
        .iter()
        .map(|word| match word.token() {
            Token::Param(_) => ObservedToken::MacroMatch,
            token => observed_macro_token(token, stores),
        })
        .collect::<Vec<_>>();
    tokens.push(ObservedToken::MacroEndMatch);
    tokens.extend(
        replacement_text
            .iter()
            .map(|word| observed_macro_token(word.token(), stores)),
    );
    tokens
}

/// §482 constructs a parameterless macro body for §1225's `define`.
pub(in crate::main_control) fn observed_read_body<G>(
    replacement_text: tex_state::TokenListId<G>,
    stores: &tex_state::CommandContext<'_, G>,
) -> Vec<ObservedToken> {
    let mut tokens = vec![ObservedToken::MacroEndMatch];
    tokens.extend(
        stores
            .token_list(replacement_text)
            .iter()
            .map(|word| observed_macro_token(word.token(), stores)),
    );
    tokens
}

pub(in crate::main_control) fn observed_macro_token<G>(
    token: Token,
    stores: &tex_state::CommandContext<'_, G>,
) -> ObservedToken {
    match token {
        // §353 gives an active character the control sequence
        // `active_base + c`, so §365's `cur_tok` stores it as
        // `cs_token_flag + cur_cs` and its §289 spelling is the single
        // character, never a character token with command code 13.
        Token::Char {
            ch,
            cat: tex_state::token::Catcode::Active,
        } => ObservedToken::ControlSequence(ch.to_string()),
        Token::Char { ch, cat } => ObservedToken::Character {
            character: ch,
            catcode: cat,
        },
        Token::Cs(symbol) => ObservedToken::ControlSequence(stores.resolve(symbol).to_owned()),
        Token::Param(slot) => ObservedToken::Parameter(slot),
        Token::Frozen(_) if token.is_frozen_end_template() => ObservedToken::FrozenEndTemplate,
        Token::Frozen(_) if token.is_frozen_endv() => ObservedToken::FrozenEndV,
        // A frozen primitive is one of tex.web's frozen control sequences, so
        // it is observed by the spelling tex.web assigns its `text`, never by
        // an engine-local slot index a transport would have to render.
        Token::Frozen(_) => stores
            .frozen_primitive_meaning(token)
            .and_then(|meaning| stores.primitive_name(meaning))
            .map(str::to_owned)
            .map_or(ObservedToken::FrozenOther, ObservedToken::FrozenPrimitive),
    }
}
