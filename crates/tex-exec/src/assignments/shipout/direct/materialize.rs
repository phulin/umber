use super::*;

pub(super) fn emitted_list_is_empty(
    stores: &Universe,
    overlay: &PageOverlay,
    list: NodeListId,
    suppress_deferred_streams: bool,
    depth: usize,
) -> Result<bool, ExecError> {
    check_depth(depth)?;
    let order = permutation_for(overlay, list);
    let len = order.map_or_else(|| stores.nodes(list).len(), <[usize]>::len);
    for position in 0..len {
        let index = order.map_or(position, |indices| indices[position]);
        if let Some(replacement) = math_substitution(overlay, list, index) {
            if !emitted_list_is_empty(
                stores,
                overlay,
                replacement,
                suppress_deferred_streams,
                depth + 1,
            )? {
                return Ok(false);
            }
            continue;
        }
        match stores
            .nodes(list)
            .get(index)
            .expect("empty scan index is live")
        {
            NodeRef::Direction(_) => {}
            NodeRef::Whatsit(Whatsit::Language { .. }) => {}
            NodeRef::Whatsit(
                Whatsit::OpenOut { .. } | Whatsit::CloseOut { .. } | Whatsit::DeferredWrite { .. },
            ) if suppress_deferred_streams => {}
            NodeRef::MathList(_) => unreachable!("phase A records every math-list substitution"),
            _ => return Ok(false),
        }
    }
    Ok(true)
}

pub(super) fn materialize_node_list(
    stores: &Universe,
    overlay: &PageOverlay,
    list: NodeListId,
    emission: &mut EmissionState,
    anchor: &mut u32,
    suppress_deferred_streams: bool,
    depth: usize,
) -> Result<Vec<PageNode>, ExecError> {
    check_depth(depth)?;
    let mut nodes = Vec::new();
    let order = permutation_for(overlay, list);
    let len = order.map_or_else(|| stores.nodes(list).len(), <[usize]>::len);
    for position in 0..len {
        let index = order.map_or(position, |indices| indices[position]);
        if let Some(replacement) = math_substitution(overlay, list, index) {
            nodes.extend(materialize_node_list(
                stores,
                overlay,
                replacement,
                emission,
                anchor,
                suppress_deferred_streams,
                depth + 1,
            )?);
            continue;
        }
        let node = stores
            .nodes(list)
            .get(index)
            .expect("leader replay index is live");
        let node = match node {
            NodeRef::Char { font, ch, .. } => {
                let (code, width) = glyph(stores, font, ch)?;
                nodes.extend(materialize_glyph(
                    stores, font, code, width, None, emission,
                )?);
                continue;
            }
            NodeRef::Lig { font, ch, orig, .. } => {
                let (code, width) = glyph(stores, font, ch)?;
                nodes.extend(materialize_glyph(
                    stores,
                    font,
                    code,
                    width,
                    Some(orig),
                    emission,
                )?);
                continue;
            }
            NodeRef::Kern { amount, kind } => Some(PageNode::Kern {
                amount,
                kind: lower_kern_kind(kind),
            }),
            NodeRef::Glue { spec, kind, leader } => Some(PageNode::Glue {
                spec: lower_glue(stores.glue(spec)),
                kind: lower_glue_kind(kind),
                leader: materialize_leader(stores, overlay, leader, emission, anchor, depth + 1)?,
            }),
            NodeRef::Penalty(value) => Some(PageNode::Penalty(value)),
            NodeRef::Rule {
                width,
                height,
                depth,
            } => Some(PageNode::Rule {
                width,
                height,
                depth,
            }),
            NodeRef::HList(box_node) | NodeRef::VList(box_node) => {
                let vertical = matches!(node, NodeRef::VList(_));
                let fields = lower_box_header(&box_node);
                let children = materialize_node_list(
                    stores,
                    overlay,
                    box_node.children,
                    emission,
                    anchor,
                    suppress_deferred_streams,
                    depth + 1,
                )?;
                let page_box = PageBoxNode { children, ..fields };
                Some(if vertical {
                    PageNode::VList(page_box)
                } else {
                    PageNode::HList(page_box)
                })
            }
            NodeRef::Disc {
                kind,
                pre,
                post,
                replace,
            } => Some(PageNode::Disc {
                kind: lower_disc_kind(kind),
                pre: materialize_node_list(
                    stores,
                    overlay,
                    pre,
                    emission,
                    anchor,
                    suppress_deferred_streams,
                    depth + 1,
                )?,
                post: materialize_node_list(
                    stores,
                    overlay,
                    post,
                    emission,
                    anchor,
                    suppress_deferred_streams,
                    depth + 1,
                )?,
                replace: materialize_node_list(
                    stores,
                    overlay,
                    replace,
                    emission,
                    anchor,
                    suppress_deferred_streams,
                    depth + 1,
                )?,
            }),
            NodeRef::Mark { class, tokens } => Some(PageNode::Mark {
                class,
                tokens: materialize_tokens(stores, tokens),
            }),
            NodeRef::Ins { class, content, .. } => Some(PageNode::Insert {
                class,
                content: materialize_node_list(
                    stores,
                    overlay,
                    content,
                    emission,
                    anchor,
                    suppress_deferred_streams,
                    depth + 1,
                )?,
            }),
            NodeRef::Whatsit(whatsit) => {
                anchor_for_whatsit(whatsit, suppress_deferred_streams, anchor)?
                    .map(|effect_index| PageNode::WhatsitAnchor { effect_index })
            }
            NodeRef::MathOn(width) => Some(PageNode::MathOn(width)),
            NodeRef::MathOff(width) => Some(PageNode::MathOff(width)),
            NodeRef::Direction(_) => None,
            NodeRef::Adjust(content) => Some(PageNode::Adjust(materialize_node_list(
                stores,
                overlay,
                content,
                emission,
                anchor,
                suppress_deferred_streams,
                depth + 1,
            )?)),
            NodeRef::Unset(_) => {
                return Err(ExecError::UnsupportedShipoutNode {
                    node: "unset alignment",
                });
            }
            NodeRef::MathList(_) => unreachable!("phase A records every math-list substitution"),
            NodeRef::MathNoad(_)
            | NodeRef::FractionNoad(_)
            | NodeRef::MathStyle(_)
            | NodeRef::MathChoice(_)
            | NodeRef::Nonscript => {
                return Err(ExecError::UnsupportedShipoutNode { node: "math" });
            }
        };
        if let Some(node) = node {
            nodes.push(node);
        }
    }
    Ok(nodes)
}

fn materialize_glyph(
    stores: &Universe,
    font: FontId,
    ch: u32,
    width: tex_state::scaled::Scaled,
    ligature_source: Option<&[char]>,
    emission: &mut EmissionState,
) -> Result<Vec<PageNode>, ExecError> {
    let projection = glyph_projection(stores, font, ch, width, emission)?;
    let mut nodes = Vec::with_capacity(3);
    if projection.left.raw() != 0 {
        nodes.push(PageNode::Kern {
            amount: projection.left,
            kind: PageKernKind::Explicit,
        });
    }
    nodes.push(match ligature_source {
        Some(source) => PageNode::Lig {
            font_id: projection.font_id,
            ch,
            source: source.iter().map(|source| *source as u32).collect(),
            width: projection.width,
        },
        None => PageNode::Char {
            font_id: projection.font_id,
            ch,
            width: projection.width,
        },
    });
    if projection.right.raw() != 0 {
        nodes.push(PageNode::Kern {
            amount: projection.right,
            kind: PageKernKind::Explicit,
        });
    }
    Ok(nodes)
}

fn materialize_leader(
    stores: &Universe,
    overlay: &PageOverlay,
    leader: Option<&StateLeaderPayload>,
    emission: &mut EmissionState,
    anchor: &mut u32,
    depth: usize,
) -> Result<Option<PageLeaderPayload>, ExecError> {
    match leader {
        None => Ok(None),
        Some(StateLeaderPayload::Rule {
            width,
            height,
            depth,
        }) => Ok(Some(PageLeaderPayload::Rule {
            width: *width,
            height: *height,
            depth: *depth,
        })),
        Some(StateLeaderPayload::HList(box_node)) | Some(StateLeaderPayload::VList(box_node)) => {
            let vertical = matches!(leader, Some(StateLeaderPayload::VList(_)));
            let fields = lower_box_header(box_node);
            let children = materialize_node_list(
                stores,
                overlay,
                box_node.children,
                emission,
                anchor,
                true,
                depth + 1,
            )?;
            let page_box = PageBoxNode { children, ..fields };
            Ok(Some(if vertical {
                PageLeaderPayload::VList(page_box)
            } else {
                PageLeaderPayload::HList(page_box)
            }))
        }
    }
}

fn materialize_tokens(stores: &Universe, list: TokenListId) -> Vec<PageToken> {
    stores
        .tokens(list)
        .iter()
        .map(|token| match *token {
            Token::Char { ch, cat } => PageToken::Char {
                ch: ch as u32,
                cat: lower_token_catcode(cat),
            },
            Token::Cs(symbol) => PageToken::ControlSequence(stores.resolve(symbol).to_owned()),
            Token::Param(slot) => PageToken::Param(slot),
            Token::Frozen(_) => unreachable!("alignment sentinel escaped into shipout tokens"),
        })
        .collect()
}
