use tex_fonts::{
    LigKernChar, LigKernCommand, LigatureCommand, MathMetricsSource, MathVariantDirection,
};
use tex_state::math::{LimitType, MathChar, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

use super::convert::{ExpandedMathView, convert_mlist};
use super::rebox::rebox;
use super::{
    BoxAxis, Context, FrozenHList, MathBox, MathNode, MathTypesetState, add, boxed_node, char_box,
    clean_box, fetch, neg, sub, variant_box,
};

#[cfg(test)]
mod tests;

pub(super) struct OperatorResult {
    pub hlist: FrozenHList,
    pub delta: Scaled,
    pub scripts_handled: bool,
}

pub(super) fn make_op(
    ctx: &mut Context<'_, impl MathTypesetState>,
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
            hlist: ctx.layout.hlist([boxed_node(nucleus)]),
            delta,
            scripts_handled: false,
        }
    }
}

pub(super) fn make_ord(
    ctx: &Context<'_, impl MathTypesetState>,
    nodes: &mut ExpandedMathView,
    index: usize,
) {
    // AppG rule 14
    loop {
        let Some((current, next)) = adjacent_math_chars(ctx, nodes, index) else {
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
        set_current_nucleus(ctx, nodes, index, MathField::MathTextChar(current));
        let Some(fetched) = fetch(ctx, current, ctx.style) else {
            // TeX82 §§722/751: `fetch` diagnoses an unavailable character
            // and sets the current nucleus to `empty`.  Leaving the text-char
            // rewrite live makes ordinary translation fetch and diagnose the
            // same field a second time before advancing to its neighbor.
            set_current_nucleus(ctx, nodes, index, MathField::Empty);
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
                nodes.insert_owned(
                    index + 1,
                    Node::Kern {
                        amount,
                        kind: KernKind::Font,
                    },
                );
                return;
            }
            LigKernCommand::Ligature(ligature) => {
                let restart = apply_math_ligature(ctx, nodes, index, ligature);
                if !restart {
                    return;
                }
            }
        }
    }
}

pub(super) fn ord_pair_may_change(
    ctx: &Context<'_, impl MathTypesetState>,
    nodes: &ExpandedMathView,
    index: usize,
) -> bool {
    adjacent_math_chars(ctx, nodes, index).is_some()
}

fn operator_nucleus(
    ctx: &mut Context<'_, impl MathTypesetState>,
    noad: &MathNoad,
    effective_limits: LimitType,
    delta: &mut Scaled,
) -> MathBox {
    let mut field = noad.nucleus;
    if let MathField::MathChar(mut ch) = field
        && ctx.style.is_display()
        && let Some(fetched) = fetch(ctx, ch, ctx.style)
        && let Ok(code) = u8::try_from(u32::from(fetched.ch))
        && let Some(next) = ctx.state.font_next_larger(fetched.font, code)
        && ctx
            .state
            .classic_math_char_metrics(fetched.font, next)
            .is_some()
    {
        ch.character = char::from(next);
        field = MathField::MathChar(ch);
    }

    match field {
        MathField::MathChar(ch) | MathField::MathTextChar(ch) => {
            let Some(fetched) = fetch(ctx, ch, ctx.style) else {
                // TeX82 §749 still reaches the common operator-centering
                // step after `fetch` reports a nonexistent math character.
                // The resulting empty hbox therefore carries the axis shift.
                // TeX82 §749's failed fetch resets the nucleus field to
                // empty before clean_box; §720 therefore returns its already-
                // clean null box without packaging it.
                let mut missing = ctx.layout.null_hbox();
                let axis = ctx.params.for_size(ctx.style.size()).symbols.axis_height;
                missing.shift = neg(axis);
                return missing;
            };
            let (mut boxed, selected_delta) = if ctx.style.is_display()
                && let MathMetricsSource::OpenType(math) =
                    ctx.state.math_metrics_source(fetched.font)
                && let Some(variant) = variant_box(
                    ctx,
                    fetched.font,
                    fetched,
                    math.display_operator_min_height(),
                    MathVariantDirection::Vertical,
                    ch.origin,
                ) {
                variant
            } else {
                (
                    char_box(ctx, fetched, ch.origin),
                    fetched.metrics.italic_correction,
                )
            };
            // TeX82 §749 sends every character operator nucleus through
            // `clean_box`; its §720 character branch completes
            // `hpack(q,natural)`. Both direct classic boxes and OpenType
            // display variants replace that call in this kernel, so publish
            // the completion after either selection path.
            ctx.layout.observe_completed_pack(&boxed);
            // The direct operator shortcut replaces both the temporary
            // one-noad §724 dimensions pack and §720's clean-box pack.
            ctx.layout.observe_completed_pack(&boxed);
            *delta = selected_delta;
            if !matches!(effective_limits, LimitType::Limits)
                && !matches!(noad.subscript, MathField::Empty)
            {
                boxed.width = sub(boxed.width, *delta);
            }
            let axis = ctx.params.for_size(ctx.style.size()).symbols.axis_height;
            boxed.shift = sub(
                Scaled::from_raw(tex_arith::half(sub(boxed.height, boxed.depth).raw())),
                axis,
            );
            boxed
        }
        MathField::SubMlist(list) => {
            // TeX.web's mlist2 branch always hpacks a non-limits operator's
            // sub-mlist nucleus. `clean_box` would incorrectly reuse a sole
            // unshifted box and remove a DVI-visible structural level.
            let list = convert_mlist(ctx, list, ctx.style, false);
            ctx.layout.hpack(list)
        }
        _ => clean_box(ctx, &field, ctx.style),
    }
}

fn displayed_limits(
    ctx: &mut Context<'_, impl MathTypesetState>,
    noad: &MathNoad,
    nucleus: MathBox,
    delta: Scaled,
) -> FrozenHList {
    // AppG rule 13a
    let size_params = ctx.params.for_size(ctx.style.size()).extension;
    let sup_style = ctx.style.sup_style();
    let sub_style = ctx.style.sub_style();
    let mut sup = clean_box(ctx, &noad.superscript, sup_style);
    let mut op = nucleus;
    if op.shift.raw() != 0 {
        let list = ctx.layout.hlist([boxed_node(op)]);
        op = ctx.layout.hpack(list);
    }
    let mut sub_box = clean_box(ctx, &noad.subscript, sub_style);
    let width = sup.width.max(op.width).max(sub_box.width);
    rebox(ctx, &mut sup, width);
    rebox(ctx, &mut op, width);
    rebox(ctx, &mut sub_box, width);
    let skew = Scaled::from_raw(tex_arith::half(delta.raw()));
    sup.shift = skew;
    sub_box.shift = neg(skew);

    let mut height = op.height;
    let mut depth = op.depth;
    let mut list = Vec::new();
    if !matches!(noad.superscript, MathField::Empty) {
        let shift_up = sub(size_params.big_op_spacing3, sup.depth).max(size_params.big_op_spacing1);
        let sup_extent = add(add(sup.height, sup.depth), shift_up);
        list.push(MathNode::Kern {
            amount: size_params.big_op_spacing5,
            kind: KernKind::Font,
        });
        list.push(boxed_node(sup));
        list.push(MathNode::Kern {
            amount: shift_up,
            kind: KernKind::Font,
        });
        height = add(height, add(size_params.big_op_spacing5, sup_extent));
    }
    list.push(boxed_node(op));
    if !matches!(noad.subscript, MathField::Empty) {
        let shift_down =
            sub(size_params.big_op_spacing4, sub_box.height).max(size_params.big_op_spacing2);
        let sub_extent = add(add(sub_box.height, sub_box.depth), shift_down);
        list.push(MathNode::Kern {
            amount: shift_down,
            kind: KernKind::Font,
        });
        list.push(boxed_node(sub_box));
        list.push(MathNode::Kern {
            amount: size_params.big_op_spacing5,
            kind: KernKind::Font,
        });
        depth = add(depth, add(size_params.big_op_spacing5, sub_extent));
    }
    let list = ctx.layout.hlist(list);
    let limits = MathBox {
        width,
        height,
        depth,
        shift: Scaled::from_raw(0),
        list,
        axis: BoxAxis::Vertical,
        display: false,
        glue_set: tex_state::scaled::GlueSetRatio::from_raw(0),
        glue_sign: tex_state::node::Sign::Normal,
        glue_order: tex_state::glue::Order::Normal,
        source: None,
    };
    ctx.layout.hlist([MathNode::VList(limits)])
}

fn adjacent_math_chars(
    ctx: &Context<'_, impl MathTypesetState>,
    nodes: &ExpandedMathView,
    index: usize,
) -> Option<(MathChar, MathChar)> {
    let Node::MathNoad(current) = nodes.node(ctx.state, index)? else {
        return None;
    };
    if !matches!(current.kind, NoadKind::Normal(NoadClass::Ord))
        || !matches!(current.subscript, MathField::Empty)
        || !matches!(current.superscript, MathField::Empty)
    {
        return None;
    }
    let current_char = math_char_field(&current.nucleus)?;
    let Node::MathNoad(next) = nodes.node(ctx.state, index + 1)? else {
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

fn set_current_nucleus(
    ctx: &Context<'_, impl MathTypesetState>,
    nodes: &mut ExpandedMathView,
    index: usize,
    field: MathField,
) {
    let Some(noad) = nodes.noad_mut(ctx.state, index) else {
        return;
    };
    noad.nucleus = field;
}

fn apply_math_ligature(
    ctx: &Context<'_, impl MathTypesetState>,
    nodes: &mut ExpandedMathView,
    index: usize,
    ligature: LigatureCommand,
) -> bool {
    let replacement = char::from(ligature.replacement);
    let restart = ligature.pass_over == 0;
    let Some(Node::MathNoad(current)) = nodes.node(ctx.state, index).cloned() else {
        return false;
    };
    let Some(current_char) = math_char_field(&current.nucleus).or(match current.nucleus {
        MathField::MathTextChar(ch) => Some(ch),
        _ => None,
    }) else {
        return false;
    };
    let replacement_field = |family| {
        let ch = MathChar {
            family,
            character: replacement,
            origin: current_char.origin,
        };
        if restart {
            MathField::MathChar(ch)
        } else {
            MathField::MathTextChar(ch)
        }
    };

    match (ligature.delete_current, ligature.delete_next) {
        (true, true) => {
            let Some(Node::MathNoad(next)) = nodes.node(ctx.state, index + 1).cloned() else {
                return false;
            };
            if let Some(current) = nodes.noad_mut(ctx.state, index) {
                current.nucleus = replacement_field(current_char.family);
                current.subscript = next.subscript;
                current.superscript = next.superscript;
            }
            nodes.remove(index + 1);
        }
        (true, false) => {
            if let Some(current) = nodes.noad_mut(ctx.state, index) {
                current.nucleus = replacement_field(current_char.family);
            }
        }
        (false, true) => {
            let Some(next) = nodes.noad_mut(ctx.state, index + 1) else {
                return false;
            };
            next.nucleus = MathField::MathChar(MathChar {
                family: current_char.family,
                character: replacement,
                origin: current_char.origin,
            });
            if restart {
                set_current_nucleus(ctx, nodes, index, MathField::MathChar(current_char));
            }
        }
        (false, false) => {
            let inserted = MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                if ligature.pass_over < 2 {
                    MathField::MathChar(MathChar {
                        family: current_char.family,
                        character: replacement,
                        origin: current_char.origin,
                    })
                } else {
                    MathField::MathTextChar(MathChar {
                        family: current_char.family,
                        character: replacement,
                        origin: current_char.origin,
                    })
                },
            );
            nodes.insert_owned(index + 1, Node::MathNoad(inserted));
            if restart {
                set_current_nucleus(ctx, nodes, index, MathField::MathChar(current_char));
            }
        }
    }
    restart
}
