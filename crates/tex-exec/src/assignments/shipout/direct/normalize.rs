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

pub(super) fn normalize_page<R: ReadRecorder>(
    root: NodeListId,
    effects: Vec<PageEffect>,
    stores: &mut Universe,
    recorder: &mut R,
) -> Result<PageOverlay, ExecError> {
    let pending_effect_count = effects.len();
    let mut overlay = PageOverlay {
        pending_effect_count,
        effects,
        math: Vec::new(),
        directions: Vec::new(),
    };
    normalize_list(stores, recorder, root, false, 1, &mut overlay)?;
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

fn normalize_list<R: ReadRecorder>(
    stores: &mut Universe,
    recorder: &mut R,
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
                recorder,
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
                recorder,
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

fn normalize_index<R: ReadRecorder>(
    stores: &mut Universe,
    recorder: &mut R,
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
            normalize_list(stores, recorder, child, suppress, depth + 1, overlay)?;
        }
        NormalizeNode::Lists(children) => {
            for child in children {
                normalize_list(
                    stores,
                    recorder,
                    child,
                    suppress_deferred_streams,
                    depth + 1,
                    overlay,
                )?;
            }
        }
        NormalizeNode::Whatsit(whatsit) => append_whatsit_effect(
            stores,
            recorder,
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
                recorder,
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

fn append_whatsit_effect<R: ReadRecorder>(
    stores: &mut Universe,
    recorder: &mut R,
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
            let text = expand_write_tokens(stores, recorder, tokens)?;
            stores.world_mut().write_text(sink, &text);
            effects.push(PageEffect::Write {
                sink: lower_sink(sink),
                text,
            });
        }
        Whatsit::Special { class, payload } => {
            effects.push(PageEffect::Special { class, payload });
        }
        Whatsit::OpenOut { .. }
        | Whatsit::CloseOut { .. }
        | Whatsit::DeferredWrite { .. }
        | Whatsit::Language { .. } => {}
    }
    Ok(())
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

fn expand_write_tokens<R: ReadRecorder>(
    stores: &mut Universe,
    recorder: &mut R,
    tokens: TokenListId,
) -> Result<String, ExecError> {
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(tokens, TokenListReplayKind::Inserted);
    let mut context = tex_expand::ExpansionContext::new("texput");
    let mut text = String::new();
    while let Some(token) =
        get_x_token_with_recorder_and_context(&mut input, stores, recorder, &mut context)?
            .map(tex_expand::semantic_token)
    {
        diagnostics::append_token_show_text(stores, token, &mut text);
    }
    let mut text = crate::diagnostics::print_text_with_newlinechar(stores, &text);
    text.push('\n');
    Ok(text)
}
