use super::*;
use tex_lex::MemoryInput;

pub(super) struct PageOverlay {
    pub(super) pending_effect_count: usize,
    pub(super) effects: Vec<PageEffect>,
    pub(super) math: Vec<MathSubstitution>,
    pub(super) directions: Vec<DirectionPermutation>,
    pub(super) diagnostics: Vec<(PrintSink, String)>,
    color_target: tex_state::PdfColorStackTarget,
    running_thread_depth: Option<usize>,
}

pub(super) struct MathSubstitution {
    pub(super) list: NodeListId,
    pub(super) index: usize,
    pub(super) replacement: NodeListId,
}

pub(super) struct DirectionPermutation {
    pub(super) list: NodeListId,
    pub(super) order: Vec<usize>,
}

pub(super) fn normalize_page(
    root: NodeListId,
    root_vertical: bool,
    effects: Vec<PageEffect>,
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
    color_target: tex_state::PdfColorStackTarget,
) -> Result<PageOverlay, ExecError> {
    let mut effects = effects;
    let snap_reference = if color_target == tex_state::PdfColorStackTarget::Page {
        stores.pdf_snap_reference()
    } else {
        (
            tex_state::scaled::Scaled::from_raw(0),
            tex_state::scaled::Scaled::from_raw(0),
        )
    };
    if snap_reference
        != (
            tex_state::scaled::Scaled::from_raw(0),
            tex_state::scaled::Scaled::from_raw(0),
        )
    {
        effects.push(PageEffect::PdfSnapState {
            x: snap_reference.0,
            y: snap_reference.1,
        });
    }
    for restoration in stores
        .pdf_page_color_stack_restorations()
        .into_iter()
        .filter(|_| color_target == tex_state::PdfColorStackTarget::Page)
    {
        effects.push(PageEffect::PdfColorStack {
            mode: lower_color_stack_mode(restoration.mode),
            payload: restoration.payload,
            page_start: true,
        });
    }
    let pending_effect_count = effects.len();
    let mut overlay = PageOverlay {
        pending_effect_count,
        effects,
        math: Vec::new(),
        directions: Vec::new(),
        diagnostics: Vec::new(),
        color_target,
        running_thread_depth: None,
    };
    normalize_list(
        stores,
        expansion,
        root,
        false,
        !root_vertical,
        1,
        &mut overlay,
    )?;
    Ok(overlay)
}

enum NormalizeNode {
    Leaf,
    List(NodeListId, bool, bool),
    Lists([NodeListId; 3]),
    Whatsit(Whatsit),
    Math(tex_state::math::MathListNode),
    Unsupported(&'static str),
}

#[derive(Clone, Copy)]
struct NormalizeLocation {
    in_hlist: bool,
    depth: usize,
}

fn normalize_list(
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
    list: NodeListId,
    suppress_deferred_streams: bool,
    in_hlist: bool,
    depth: usize,
    overlay: &mut PageOverlay,
) -> Result<(), ExecError> {
    check_depth(depth)?;
    let permutation = direction_permutation(stores, list);
    if let Some(order) = permutation.as_ref() {
        overlay.directions.push(DirectionPermutation {
            list,
            order: order.clone(),
        });
        for &index in order {
            normalize_index(
                stores,
                expansion,
                list,
                index,
                suppress_deferred_streams,
                NormalizeLocation { in_hlist, depth },
                overlay,
            )?;
        }
    } else {
        let len = stores.nodes(list).len();
        for index in 0..len {
            normalize_index(
                stores,
                expansion,
                list,
                index,
                suppress_deferred_streams,
                NormalizeLocation { in_hlist, depth },
                overlay,
            )?;
        }
    }
    Ok(())
}

fn normalize_index(
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
    list: NodeListId,
    index: usize,
    suppress_deferred_streams: bool,
    location: NormalizeLocation,
    overlay: &mut PageOverlay,
) -> Result<(), ExecError> {
    let NormalizeLocation { in_hlist, depth } = location;
    let action = {
        let node = stores
            .nodes(list)
            .get(index)
            .expect("normalization index belongs to the frozen list");
        match node {
            NodeRef::HList(box_node) => {
                NormalizeNode::List(box_node.children, suppress_deferred_streams, true)
            }
            NodeRef::VList(box_node) => {
                NormalizeNode::List(box_node.children, suppress_deferred_streams, false)
            }
            NodeRef::Glue {
                leader: Some(StateLeaderPayload::HList(box_node)),
                ..
            } => NormalizeNode::List(box_node.children, true, true),
            NodeRef::Glue {
                leader: Some(StateLeaderPayload::VList(box_node)),
                ..
            } => NormalizeNode::List(box_node.children, true, false),
            NodeRef::Disc {
                pre, post, replace, ..
            } => NormalizeNode::Lists([pre, post, replace]),
            NodeRef::Ins { content, .. } | NodeRef::Adjust(content) => {
                NormalizeNode::List(content, suppress_deferred_streams, in_hlist)
            }
            NodeRef::Whatsit(whatsit) => NormalizeNode::Whatsit(whatsit.clone()),
            NodeRef::MathList(math) => NormalizeNode::Math(math),
            NodeRef::Unset(_) => NormalizeNode::Unsupported("unset alignment"),
            NodeRef::MathNoad(_)
            | NodeRef::FractionNoad(_)
            | NodeRef::MathStyle(_)
            | NodeRef::MathChoice(_)
            | NodeRef::Nonscript => NormalizeNode::Unsupported("math"),
            NodeRef::Char { .. }
            | NodeRef::Lig { .. }
            | NodeRef::Kern { .. }
            | NodeRef::Glue { .. }
            | NodeRef::Penalty(_)
            | NodeRef::Rule { .. }
            | NodeRef::Mark { .. }
            | NodeRef::MathOn(_)
            | NodeRef::MathOff(_)
            | NodeRef::Direction(_) => NormalizeNode::Leaf,
        }
    };
    match action {
        NormalizeNode::Leaf => {}
        NormalizeNode::List(child, suppress, child_in_hlist) => {
            normalize_list(
                stores,
                expansion,
                child,
                suppress,
                child_in_hlist,
                depth + 1,
                overlay,
            )?;
        }
        NormalizeNode::Lists(children) => {
            for child in children {
                normalize_list(
                    stores,
                    expansion,
                    child,
                    suppress_deferred_streams,
                    in_hlist,
                    depth + 1,
                    overlay,
                )?;
            }
        }
        NormalizeNode::Whatsit(whatsit) => append_whatsit_effect(
            stores,
            expansion,
            overlay,
            whatsit,
            suppress_deferred_streams,
            location,
        )?,
        NormalizeNode::Math(math) => {
            let mut nodes = crate::math::finish_math_list_node(stores, math, false);
            let replacement = stores.freeze_node_list_owned(&mut nodes);
            overlay.math.push(MathSubstitution {
                list,
                index,
                replacement,
            });
            normalize_list(
                stores,
                expansion,
                replacement,
                suppress_deferred_streams,
                in_hlist,
                depth + 1,
                overlay,
            )?;
        }
        NormalizeNode::Unsupported(node) => {
            return Err(ExecError::UnsupportedShipoutNode { node });
        }
    }
    Ok(())
}

fn append_whatsit_effect(
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
    overlay: &mut PageOverlay,
    whatsit: Whatsit,
    suppress_deferred_streams: bool,
    location: NormalizeLocation,
) -> Result<(), ExecError> {
    let NormalizeLocation { in_hlist, depth } = location;
    let color_target = overlay.color_target;
    let effects = &mut overlay.effects;
    let diagnostics = &mut overlay.diagnostics;
    let running_thread_depth = &mut overlay.running_thread_depth;
    match whatsit {
        Whatsit::OpenOut { slot, path } if !suppress_deferred_streams => {
            let path = super::super::super::variables::openout_target(path);
            stores.world_mut().open_out(slot, path.clone());
            effects.push(PageEffect::OpenOut {
                stream: slot.raw(),
                path,
            });
        }
        Whatsit::CloseOut { slot } if !suppress_deferred_streams => {
            stores.world_mut().close_out(slot);
            effects.push(PageEffect::CloseOut { stream: slot.raw() });
        }
        Whatsit::DeferredWrite { sink, tokens } if !suppress_deferred_streams => {
            let text = expand_write_tokens(stores, expansion, tokens)?;
            stores.world_mut().write_text(sink, &text);
            effects.push(PageEffect::Write {
                sink: lower_sink(sink),
                text,
            });
        }
        Whatsit::Special { class, payload } => {
            effects.push(PageEffect::Special { class, payload });
        }
        Whatsit::PdfReferenceObject { object } => {
            stores
                .reference_pdf_raw_object(object)
                .map_err(|_| ExecError::PdfReferencedObjectNotFound)?;
        }
        Whatsit::PdfAccessibility(control) => {
            effects.push(PageEffect::PdfAccessibility(match control {
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOn => {
                    tex_out::PdfAccessibilityEffect::InterwordSpaceOn
                }
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOff => {
                    tex_out::PdfAccessibilityEffect::InterwordSpaceOff
                }
                tex_state::node::PdfAccessibilityControl::FakeSpace => {
                    tex_out::PdfAccessibilityEffect::FakeSpace
                }
            }));
        }
        Whatsit::PdfAnnotation { object } => {
            effects.push(PageEffect::PdfAnnotation(
                tex_out::PdfAnnotationEffect::Annotation { object },
            ));
        }
        Whatsit::PdfLinkStart { object } => {
            effects.push(PageEffect::PdfAnnotation(
                tex_out::PdfAnnotationEffect::LinkStart { object },
            ));
        }
        Whatsit::PdfLinkEnd { object } => {
            effects.push(PageEffect::PdfAnnotation(
                tex_out::PdfAnnotationEffect::LinkEnd { object },
            ));
        }
        Whatsit::PdfRunningLink(enabled) => {
            effects.push(PageEffect::PdfAnnotation(
                tex_out::PdfAnnotationEffect::RunningLink(enabled),
            ));
        }
        Whatsit::PdfLiteral { mode, payload } => effects.push(PageEffect::PdfLiteral {
            mode: lower_pdf_literal_mode(mode),
            payload,
        }),
        Whatsit::DeferredPdfLiteral { mode, tokens } => {
            let payload = expand_pdf_literal_tokens(stores, expansion, tokens)?;
            effects.push(PageEffect::PdfLiteral {
                mode: lower_pdf_literal_mode(mode),
                payload,
            });
        }
        Whatsit::PdfSetMatrix { payload } => {
            validate_pdf_matrix(&payload)?;
            effects.push(PageEffect::PdfSetMatrix { payload });
        }
        Whatsit::PdfSave => effects.push(PageEffect::PdfSave),
        Whatsit::PdfRestore => effects.push(PageEffect::PdfRestore),
        Whatsit::PdfColorStack { id, action } => {
            match stores.apply_pdf_color_stack(id, color_target, &action) {
                Ok(emission) => effects.push(PageEffect::PdfColorStack {
                    mode: lower_color_stack_mode(emission.mode),
                    payload: emission.payload,
                    page_start: false,
                }),
                Err(tex_state::PdfColorStackApplyError::Underflow) => {
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        &format!("pop empty color page stack {id}\n"),
                    );
                    // Preserve the artifact anchor correspondence without writing
                    // any content-stream bytes for the failed pop.
                    effects.push(PageEffect::PdfColorStack {
                        mode: tex_out::PdfLiteralMode::Direct,
                        payload: Vec::new(),
                        page_start: false,
                    });
                }
                Err(tex_state::PdfColorStackApplyError::Unknown) => {
                    unreachable!("validated color stack id")
                }
            }
        }
        Whatsit::PdfSavePos => effects.push(PageEffect::PdfSavePosition),
        Whatsit::PdfSnapRefPoint => effects.push(PageEffect::PdfSnapRefPoint),
        Whatsit::PdfSnapY { glue } => effects.push(PageEffect::PdfSnapY {
            spec: super::lower_glue(stores.glue(glue)),
        }),
        Whatsit::PdfSnapYComp { ratio } => effects.push(PageEffect::PdfSnapYComp { ratio }),
        Whatsit::PdfRefXForm {
            object,
            width,
            height,
            depth,
        } => {
            if stores.pdf_form_artifact(object).is_none() {
                let form = stores
                    .pdf_form(object)
                    .ok_or(ExecError::PdfReferencedObjectNotFound)?;
                let artifact = super::stage_form(form, stores, expansion)?;
                stores.publish_pdf_traversal_positions(
                    artifact.last_position(),
                    stores.pdf_snap_reference(),
                );
                stores.set_pdf_form_artifact(object, artifact);
            }
            effects.push(PageEffect::PdfRefXForm {
                object,
                width,
                height,
                depth,
            });
        }
        Whatsit::PdfRefXImage {
            object,
            width,
            height,
            depth,
        } => effects.push(PageEffect::PdfRefXImage {
            object,
            width,
            height,
            depth,
        }),
        Whatsit::PdfDestination {
            identifier,
            structure,
            kind,
        } => {
            if suppress_deferred_streams {
                return Ok(());
            }
            if color_target == tex_state::PdfColorStackTarget::Form {
                return Err(ExecError::PdfDestinationInForm);
            }
            let identity = match identifier {
                tex_state::PdfActionIdentifier::Name(tokens) => {
                    let mut text = String::new();
                    for &token in stores.tokens(tokens) {
                        tex_expand::append_token_string_text(stores, token, &mut text);
                    }
                    tex_state::PdfDestinationIdentity::Name(text.into_bytes())
                }
                tex_state::PdfActionIdentifier::Number(number) => {
                    tex_state::PdfDestinationIdentity::Number(number)
                }
                tex_state::PdfActionIdentifier::Raw(_) => {
                    unreachable!("destination scanner uses typed identifiers")
                }
            };
            let definition = stores
                .define_pdf_destination(identity.clone(), structure)
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            if definition.duplicate
                && stores.int_param(IntParam::PDF_SUPPRESS_WARNING_DUP_DEST) <= 0
            {
                diagnostics.push((
                    PrintSink::TerminalAndLog,
                    super::super::super::pdf_destination_duplicate_warning(&identity),
                ));
            }
            let identifier = match identity {
                tex_state::PdfDestinationIdentity::Name(name) => {
                    tex_out::PdfDestinationIdentifier::Name(name)
                }
                tex_state::PdfDestinationIdentity::Number(number) => {
                    tex_out::PdfDestinationIdentifier::Number(number)
                }
            };
            let kind = match kind {
                tex_state::node::PdfDestinationKind::Xyz { zoom } => {
                    tex_out::PdfDestinationKind::Xyz { zoom }
                }
                tex_state::node::PdfDestinationKind::FitBoundingBoxHorizontal => {
                    tex_out::PdfDestinationKind::FitBoundingBoxHorizontal
                }
                tex_state::node::PdfDestinationKind::FitBoundingBoxVertical => {
                    tex_out::PdfDestinationKind::FitBoundingBoxVertical
                }
                tex_state::node::PdfDestinationKind::FitBoundingBox => {
                    tex_out::PdfDestinationKind::FitBoundingBox
                }
                tex_state::node::PdfDestinationKind::FitHorizontal => {
                    tex_out::PdfDestinationKind::FitHorizontal
                }
                tex_state::node::PdfDestinationKind::FitVertical => {
                    tex_out::PdfDestinationKind::FitVertical
                }
                tex_state::node::PdfDestinationKind::FitRectangle(dimensions) => {
                    tex_out::PdfDestinationKind::FitRectangle {
                        width: dimensions.width,
                        height: dimensions.height,
                        depth: dimensions.depth,
                    }
                }
                tex_state::node::PdfDestinationKind::Fit => tex_out::PdfDestinationKind::Fit,
            };
            effects.push(PageEffect::PdfDestination(tex_out::PdfDestinationEffect {
                object: definition.record.object(),
                identifier,
                structure,
                kind,
                margin: stores.dimen_param(DimenParam::PDF_DEST_MARGIN),
            }));
        }
        Whatsit::PdfThread {
            identifier,
            dimensions,
            attributes,
            running,
        } => {
            if suppress_deferred_streams {
                return Ok(());
            }
            if running && in_hlist {
                diagnostics.push((
                    PrintSink::TerminalAndLog,
                    "\npdfTeX warning: \\pdfstartthread ended up in hlist\n".to_owned(),
                ));
                effects.push(PageEffect::PdfLiteral {
                    mode: tex_out::PdfLiteralMode::Direct,
                    payload: Vec::new(),
                });
                return Ok(());
            }
            if color_target == tex_state::PdfColorStackTarget::Form {
                return Err(ExecError::PdfThreadInForm);
            }
            let identity = match identifier {
                tex_state::PdfActionIdentifier::Name(tokens) => {
                    let mut text = String::new();
                    for &token in stores.tokens(tokens) {
                        tex_expand::append_token_string_text(stores, token, &mut text);
                    }
                    tex_state::PdfDestinationIdentity::Name(text.into_bytes())
                }
                tex_state::PdfActionIdentifier::Number(number) => {
                    tex_state::PdfDestinationIdentity::Number(number)
                }
                tex_state::PdfActionIdentifier::Raw(_) => {
                    unreachable!("thread scanner uses typed identifiers")
                }
            };
            let (thread, bead) = stores
                .append_pdf_thread_bead(identity.clone())
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            let identifier = match identity {
                tex_state::PdfDestinationIdentity::Name(name) => {
                    tex_out::PdfDestinationIdentifier::Name(name)
                }
                tex_state::PdfDestinationIdentity::Number(number) => {
                    tex_out::PdfDestinationIdentifier::Number(number)
                }
            };
            let mut attribute_bytes = String::new();
            for &token in stores.tokens(attributes) {
                tex_expand::append_token_string_text(stores, token, &mut attribute_bytes);
            }
            let marker = tex_out::PdfThreadEffect {
                thread_object: thread.object(),
                bead_object: bead.bead_object(),
                rectangle_object: bead.rectangle_object(),
                identifier,
                width: dimensions.width,
                height: dimensions.height,
                depth: dimensions.depth,
                attributes: attribute_bytes.into_bytes(),
                margin: stores.dimen_param(DimenParam::PDF_THREAD_MARGIN),
            };
            if running {
                *running_thread_depth = Some(depth);
            }
            effects.push(if running {
                PageEffect::PdfStartThread(marker)
            } else {
                PageEffect::PdfThread(marker)
            });
        }
        Whatsit::PdfEndThread if in_hlist => {
            diagnostics.push((
                PrintSink::TerminalAndLog,
                "\npdfTeX warning: \\pdfendthread ended up in hlist\n".to_owned(),
            ));
            effects.push(PageEffect::PdfLiteral {
                mode: tex_out::PdfLiteralMode::Direct,
                payload: Vec::new(),
            });
        }
        Whatsit::PdfEndThread => match running_thread_depth.take() {
            Some(start_depth) if start_depth != depth => {
                diagnostics.push((
                    PrintSink::TerminalAndLog,
                    "\npdfTeX warning: \\pdfendthread ended up in different nesting level than \\pdfstartthread\n"
                        .to_owned(),
                ));
                effects.push(PageEffect::PdfLiteral {
                    mode: tex_out::PdfLiteralMode::Direct,
                    payload: Vec::new(),
                });
            }
            _ => effects.push(PageEffect::PdfEndThread),
        },
        Whatsit::OpenOut { .. }
        | Whatsit::CloseOut { .. }
        | Whatsit::DeferredWrite { .. }
        | Whatsit::Language { .. } => {}
    }
    Ok(())
}

fn validate_pdf_matrix(payload: &[u8]) -> Result<(), ExecError> {
    let valid = std::str::from_utf8(payload).ok().is_some_and(|text| {
        let mut fields = text.split_ascii_whitespace();
        let four_finite = (0..4).all(|_| {
            fields
                .next()
                .and_then(|field| field.parse::<f64>().ok())
                .is_some_and(f64::is_finite)
        });
        four_finite && fields.next().is_none()
    });
    if valid {
        Ok(())
    } else {
        Err(ExecError::InvalidShipoutArtifact(
            "pdfTeX error (\\pdfsetmatrix): Unrecognized format.".to_owned(),
        ))
    }
}

fn lower_pdf_literal_mode(mode: tex_state::node::PdfLiteralMode) -> tex_out::PdfLiteralMode {
    match mode {
        tex_state::node::PdfLiteralMode::Origin => tex_out::PdfLiteralMode::Origin,
        tex_state::node::PdfLiteralMode::Page => tex_out::PdfLiteralMode::Page,
        tex_state::node::PdfLiteralMode::Direct => tex_out::PdfLiteralMode::Direct,
    }
}

fn lower_color_stack_mode(mode: tex_state::PdfColorStackMode) -> tex_out::PdfLiteralMode {
    match mode {
        tex_state::PdfColorStackMode::Origin => tex_out::PdfLiteralMode::Origin,
        tex_state::PdfColorStackMode::Page => tex_out::PdfLiteralMode::Page,
        tex_state::PdfColorStackMode::Direct => tex_out::PdfLiteralMode::Direct,
    }
}

fn expand_pdf_literal_tokens(
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
    tokens: TokenListId,
) -> Result<Vec<u8>, ExecError> {
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(tokens, TokenListReplayKind::Inserted);
    let mut text = String::new();
    while let Some(token) = get_x_or_protected_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(stores),
        expansion,
    )?
    .map(tex_expand::semantic_token)
    {
        diagnostics::append_token_show_text(stores, token, &mut text);
    }
    Ok(text.into_bytes())
}

fn direction_permutation(stores: &Universe, list: NodeListId) -> Option<Vec<usize>> {
    struct Segment {
        right_to_left: bool,
        chunks: Vec<Vec<usize>>,
    }
    fn append(target: &mut Vec<usize>, stack: &mut [Segment], index: usize) {
        if let Some(segment) = stack.last_mut() {
            segment.chunks.push(vec![index]);
        } else {
            target.push(index);
        }
    }
    fn finish(target: &mut Vec<usize>, stack: &mut Vec<Segment>) {
        let Some(mut segment) = stack.pop() else {
            return;
        };
        if segment.right_to_left {
            segment.chunks.reverse();
        }
        let nodes = segment.chunks.into_iter().flatten().collect::<Vec<_>>();
        if let Some(parent) = stack.last_mut() {
            parent.chunks.push(nodes);
        } else {
            target.extend(nodes);
        }
    }

    let nodes = stores.nodes(list);
    if !nodes.contains_direction() {
        return None;
    }
    let mut reordered = Vec::with_capacity(nodes.len());
    let mut stack = Vec::<Segment>::new();
    for (index, node) in nodes.into_iter().enumerate() {
        match node {
            NodeRef::Direction(Direction::BeginL) => stack.push(Segment {
                right_to_left: false,
                chunks: Vec::new(),
            }),
            NodeRef::Direction(Direction::BeginR) => stack.push(Segment {
                right_to_left: true,
                chunks: Vec::new(),
            }),
            NodeRef::Direction(Direction::EndL)
                if stack.last().is_some_and(|segment| !segment.right_to_left) =>
            {
                finish(&mut reordered, &mut stack);
            }
            NodeRef::Direction(Direction::EndR)
                if stack.last().is_some_and(|segment| segment.right_to_left) =>
            {
                finish(&mut reordered, &mut stack);
            }
            NodeRef::Direction(_) => {}
            _ => append(&mut reordered, &mut stack, index),
        }
    }
    while !stack.is_empty() {
        finish(&mut reordered, &mut stack);
    }
    Some(reordered)
}

fn expand_write_tokens(
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
    tokens: TokenListId,
) -> Result<String, ExecError> {
    let mut input = InputStack::empty();
    input.push_token_list(tokens, TokenListReplayKind::Inserted);
    let mut text = String::new();
    while let Some(token) = get_x_or_protected_with_context(
        &mut input,
        &mut tex_state::ExpansionContext::new(stores),
        expansion,
    )?
    .map(tex_expand::semantic_token)
    {
        diagnostics::append_token_show_text(stores, token, &mut text);
    }
    let mut text = crate::diagnostics::print_text_with_newlinechar(stores, &text);
    text.push('\n');
    Ok(text)
}
