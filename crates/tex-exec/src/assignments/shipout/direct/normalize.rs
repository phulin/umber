use super::*;

pub(super) struct PageOverlay {
    pub(super) pending_effect_count: usize,
    pub(super) effects: Vec<PageEffect>,
    pub(super) math: Vec<MathSubstitution>,
    pub(super) directions: Vec<DirectionPermutation>,
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
    effects: Vec<PageEffect>,
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
) -> Result<PageOverlay, ExecError> {
    let pending_effect_count = effects.len();
    let mut overlay = PageOverlay {
        pending_effect_count,
        effects,
        math: Vec::new(),
        directions: Vec::new(),
    };
    normalize_list(stores, expansion, root, false, 1, &mut overlay)?;
    Ok(overlay)
}

enum NormalizeNode {
    Leaf,
    List(NodeListId, bool),
    Lists([NodeListId; 3]),
    Whatsit(Whatsit),
    Math(tex_state::math::MathListNode),
    Unsupported(&'static str),
}

fn normalize_list(
    stores: &mut Universe,
    expansion: &mut tex_expand::ExpansionContext<'_>,
    list: NodeListId,
    suppress_deferred_streams: bool,
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
                depth,
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
                depth,
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
    depth: usize,
    overlay: &mut PageOverlay,
) -> Result<(), ExecError> {
    let action = {
        let node = stores
            .nodes(list)
            .get(index)
            .expect("normalization index belongs to the frozen list");
        match node {
            NodeRef::HList(box_node) | NodeRef::VList(box_node) => {
                NormalizeNode::List(box_node.children, suppress_deferred_streams)
            }
            NodeRef::Glue {
                leader: Some(StateLeaderPayload::HList(box_node)),
                ..
            }
            | NodeRef::Glue {
                leader: Some(StateLeaderPayload::VList(box_node)),
                ..
            } => NormalizeNode::List(box_node.children, true),
            NodeRef::Disc {
                pre, post, replace, ..
            } => NormalizeNode::Lists([pre, post, replace]),
            NodeRef::Ins { content, .. } | NodeRef::Adjust(content) => {
                NormalizeNode::List(content, suppress_deferred_streams)
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
        NormalizeNode::List(child, suppress) => {
            normalize_list(stores, expansion, child, suppress, depth + 1, overlay)?;
        }
        NormalizeNode::Lists(children) => {
            for child in children {
                normalize_list(
                    stores,
                    expansion,
                    child,
                    suppress_deferred_streams,
                    depth + 1,
                    overlay,
                )?;
            }
        }
        NormalizeNode::Whatsit(whatsit) => append_whatsit_effect(
            stores,
            expansion,
            &mut overlay.effects,
            whatsit,
            suppress_deferred_streams,
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
    effects: &mut Vec<PageEffect>,
    whatsit: Whatsit,
    suppress_deferred_streams: bool,
) -> Result<(), ExecError> {
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
        Whatsit::PdfSetMatrix { payload } => effects.push(PageEffect::PdfSetMatrix { payload }),
        Whatsit::PdfSave => effects.push(PageEffect::PdfSave),
        Whatsit::PdfRestore => effects.push(PageEffect::PdfRestore),
        Whatsit::OpenOut { .. }
        | Whatsit::CloseOut { .. }
        | Whatsit::DeferredWrite { .. }
        | Whatsit::Language { .. } => {}
    }
    Ok(())
}

fn lower_pdf_literal_mode(mode: tex_state::node::PdfLiteralMode) -> tex_out::PdfLiteralMode {
    match mode {
        tex_state::node::PdfLiteralMode::Origin => tex_out::PdfLiteralMode::Origin,
        tex_state::node::PdfLiteralMode::Page => tex_out::PdfLiteralMode::Page,
        tex_state::node::PdfLiteralMode::Direct => tex_out::PdfLiteralMode::Direct,
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
