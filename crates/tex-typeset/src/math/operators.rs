use tex_fonts::{LigKernChar, LigKernCommand, LigatureCommand};
use tex_state::math::{LimitType, MathChar, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

use super::{
    BoxAxis, Context, FrozenHList, MathBox, MathNode, MathTypesetState, add, boxed_node, char_box,
    clean_box, fetch, hpack, sub,
};

pub(super) struct OperatorResult {
    pub hlist: FrozenHList,
    pub delta: Scaled,
    pub scripts_handled: bool,
}

pub(super) fn make_op(
    ctx: &Context<'_, impl MathTypesetState>,
    noad: &MathNoad,
    limit_type: LimitType,
) -> OperatorResult {
    // AppG rule 13
    let effective_limits =
        if matches!(limit_type, LimitType::DisplayLimits) && ctx.style.is_display() {
            LimitType::Limits
        } else {
            limit_type
        };
    let mut delta = Scaled::from_raw(0);
    let nucleus = operator_nucleus(ctx, noad, effective_limits, &mut delta);
    if matches!(effective_limits, LimitType::Limits) {
        OperatorResult {
            hlist: displayed_limits(ctx, noad, nucleus, delta),
            delta,
            scripts_handled: true,
        }
    } else {
        OperatorResult {
            hlist: FrozenHList {
                nodes: vec![boxed_node(nucleus)],
            },
            delta,
            scripts_handled: false,
        }
    }
}

pub(super) fn make_ord(
    ctx: &Context<'_, impl MathTypesetState>,
    nodes: &mut Vec<Node>,
    index: usize,
) {
    // AppG rule 14
    loop {
        let Some((current, next)) = adjacent_math_chars(nodes, index) else {
            return;
        };
        let current_code = match u8::try_from(u32::from(current.character)) {
            Ok(code) => code,
            Err(_) => return,
        };
        let next_code = match u8::try_from(u32::from(next.character)) {
            Ok(code) => code,
            Err(_) => return,
        };
        set_current_nucleus(nodes, index, MathField::MathTextChar(current));
        let Some(fetched) = fetch(ctx.state, current, ctx.style) else {
            return;
        };
        let Some(command) = ctx.state.lig_kern_command(
            fetched.font,
            LigKernChar::Char(current_code),
            LigKernChar::Char(next_code),
        ) else {
            return;
        };
        match command {
            LigKernCommand::Kern(amount) => {
                nodes.insert(
                    index + 1,
                    Node::Kern {
                        amount,
                        kind: KernKind::Font,
                    },
                );
                return;
            }
            LigKernCommand::Ligature(ligature) => {
                let restart = apply_math_ligature(nodes, index, ligature);
                if !restart {
                    return;
                }
            }
        }
    }
}

fn operator_nucleus(
    ctx: &Context<'_, impl MathTypesetState>,
    noad: &MathNoad,
    effective_limits: LimitType,
    delta: &mut Scaled,
) -> MathBox {
    let mut field = noad.nucleus.clone();
    if let MathField::MathChar(mut ch) = field
        && ctx.style.is_display()
        && let Some(fetched) = fetch(ctx.state, ch, ctx.style)
        && let Ok(code) = u8::try_from(u32::from(fetched.ch))
        && let Some(next) = ctx.state.font_next_larger(fetched.font, code)
        && ctx.state.font_char_metrics(fetched.font, next).is_some()
    {
        ch.character = char::from(next);
        field = MathField::MathChar(ch);
    }

    let mut boxed = match field {
        MathField::MathChar(ch) | MathField::MathTextChar(ch) => {
            let Some(fetched) = fetch(ctx.state, ch, ctx.style) else {
                return hpack(FrozenHList::default());
            };
            *delta = fetched.metrics.italic_correction;
            let mut boxed = char_box(fetched);
            if !matches!(effective_limits, LimitType::Limits)
                && !matches!(noad.subscript, MathField::Empty)
            {
                boxed.width = sub(boxed.width, *delta);
            }
            boxed
        }
        _ => clean_box(ctx.state, &field, ctx.style, ctx.params),
    };
    let axis = ctx.params.for_size(ctx.style.size()).symbols.axis_height;
    boxed.shift = sub(
        axis,
        Scaled::from_raw(tex_arith::half(sub(boxed.height, boxed.depth).raw())),
    );
    boxed
}

fn displayed_limits(
    ctx: &Context<'_, impl MathTypesetState>,
    noad: &MathNoad,
    nucleus: MathBox,
    delta: Scaled,
) -> FrozenHList {
    // AppG rule 13a
    let size_params = ctx.params.for_size(ctx.style.size()).extension;
    let mut sup = clean_box(
        ctx.state,
        &noad.superscript,
        ctx.style.sup_style(),
        ctx.params,
    );
    let mut op = nucleus;
    let mut sub_box = clean_box(
        ctx.state,
        &noad.subscript,
        ctx.style.sub_style(),
        ctx.params,
    );
    let width = sup.width.max(op.width).max(sub_box.width);
    rebox(&mut sup, width);
    rebox(&mut op, width);
    rebox(&mut sub_box, width);
    let op = hpack(FrozenHList {
        nodes: vec![MathNode::HList(op)],
    });
    let skew = Scaled::from_raw(tex_arith::half(delta.raw()));
    sup.shift = skew;
    sub_box.shift = Scaled::from_raw(-skew.raw());

    let mut height = op.height;
    let mut depth = op.depth;
    let mut list = Vec::new();
    if !matches!(noad.superscript, MathField::Empty) {
        let shift_up = sub(size_params.big_op_spacing3, sup.depth).max(size_params.big_op_spacing1);
        list.push(MathNode::Kern {
            amount: size_params.big_op_spacing5,
            kind: KernKind::Explicit,
        });
        list.push(MathNode::HList(sup.clone()));
        list.push(MathNode::Kern {
            amount: shift_up,
            kind: KernKind::Explicit,
        });
        height = add(
            height,
            add(
                size_params.big_op_spacing5,
                add(add(sup.height, sup.depth), shift_up),
            ),
        );
    }
    list.push(MathNode::HList(op));
    if !matches!(noad.subscript, MathField::Empty) {
        let shift_down =
            sub(size_params.big_op_spacing4, sub_box.height).max(size_params.big_op_spacing2);
        list.push(MathNode::Kern {
            amount: shift_down,
            kind: KernKind::Explicit,
        });
        list.push(MathNode::HList(sub_box.clone()));
        list.push(MathNode::Kern {
            amount: size_params.big_op_spacing5,
            kind: KernKind::Explicit,
        });
        depth = add(
            depth,
            add(
                size_params.big_op_spacing5,
                add(add(sub_box.height, sub_box.depth), shift_down),
            ),
        );
    }
    FrozenHList {
        nodes: vec![MathNode::VList(MathBox {
            width,
            height,
            depth,
            shift: Scaled::from_raw(0),
            list: FrozenHList { nodes: list },
            axis: BoxAxis::Vertical,
        })],
    }
}

fn adjacent_math_chars(nodes: &[Node], index: usize) -> Option<(MathChar, MathChar)> {
    let Node::MathNoad(current) = nodes.get(index)? else {
        return None;
    };
    if !matches!(current.kind, NoadKind::Normal(NoadClass::Ord))
        || !matches!(current.subscript, MathField::Empty)
        || !matches!(current.superscript, MathField::Empty)
    {
        return None;
    }
    let current_char = math_char_field(&current.nucleus)?;
    let Node::MathNoad(next) = nodes.get(index + 1)? else {
        return None;
    };
    if !can_follow_ord_for_lig_kern(next) {
        return None;
    }
    let next_char = math_char_field(&next.nucleus)?;
    (current_char.family == next_char.family).then_some((current_char, next_char))
}

fn math_char_field(field: &MathField) -> Option<MathChar> {
    match field {
        MathField::MathChar(ch) => Some(*ch),
        _ => None,
    }
}

fn can_follow_ord_for_lig_kern(noad: &MathNoad) -> bool {
    matches!(
        &noad.kind,
        NoadKind::Normal(NoadClass::Ord | NoadClass::Bin | NoadClass::Rel)
            | NoadKind::Normal(NoadClass::Open | NoadClass::Close | NoadClass::Punct)
            | NoadKind::Operator(_)
            | NoadKind::Normal(NoadClass::Op)
    )
}

fn set_current_nucleus(nodes: &mut [Node], index: usize, field: MathField) {
    let Some(Node::MathNoad(noad)) = nodes.get_mut(index) else {
        return;
    };
    noad.nucleus = field;
}

fn apply_math_ligature(nodes: &mut Vec<Node>, index: usize, ligature: LigatureCommand) -> bool {
    let replacement = char::from(ligature.replacement);
    let restart = ligature.pass_over == 0;
    let replacement_field = |family| {
        let ch = MathChar {
            family,
            character: replacement,
        };
        if restart {
            MathField::MathChar(ch)
        } else {
            MathField::MathTextChar(ch)
        }
    };
    let Some(Node::MathNoad(current)) = nodes.get(index).cloned() else {
        return false;
    };
    let Some(current_char) = math_char_field(&current.nucleus).or(match current.nucleus {
        MathField::MathTextChar(ch) => Some(ch),
        _ => None,
    }) else {
        return false;
    };

    match (ligature.delete_current, ligature.delete_next) {
        (true, true) => {
            let Some(Node::MathNoad(next)) = nodes.get(index + 1).cloned() else {
                return false;
            };
            if let Some(Node::MathNoad(current)) = nodes.get_mut(index) {
                current.nucleus = replacement_field(current_char.family);
                current.subscript = next.subscript;
                current.superscript = next.superscript;
            }
            nodes.remove(index + 1);
        }
        (true, false) => {
            if let Some(Node::MathNoad(current)) = nodes.get_mut(index) {
                current.nucleus = replacement_field(current_char.family);
            }
        }
        (false, true) => {
            let Some(Node::MathNoad(next)) = nodes.get_mut(index + 1) else {
                return false;
            };
            next.nucleus = MathField::MathChar(MathChar {
                family: current_char.family,
                character: replacement,
            });
            if restart {
                set_current_nucleus(nodes, index, MathField::MathChar(current_char));
            }
        }
        (false, false) => {
            let inserted = MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                if ligature.pass_over < 2 {
                    MathField::MathChar(MathChar {
                        family: current_char.family,
                        character: replacement,
                    })
                } else {
                    MathField::MathTextChar(MathChar {
                        family: current_char.family,
                        character: replacement,
                    })
                },
            );
            nodes.insert(index + 1, Node::MathNoad(inserted));
            if restart {
                set_current_nucleus(nodes, index, MathField::MathChar(current_char));
            }
        }
    }
    restart
}

fn rebox(boxed: &mut MathBox, width: Scaled) {
    // AppG rule 13a
    let slack = sub(width, boxed.width);
    if slack.raw() != 0 && matches!(boxed.axis, BoxAxis::Horizontal) {
        let left = Scaled::from_raw(tex_arith::half(slack.raw()));
        let right = sub(slack, left);
        if left.raw() != 0 {
            boxed.list.nodes.insert(
                0,
                MathNode::Kern {
                    amount: left,
                    kind: KernKind::Explicit,
                },
            );
        }
        if right.raw() != 0 {
            boxed.list.nodes.push(MathNode::Kern {
                amount: right,
                kind: KernKind::Explicit,
            });
        }
    }
    boxed.width = width;
}
