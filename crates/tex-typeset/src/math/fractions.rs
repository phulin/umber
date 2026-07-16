use tex_state::math::{FractionThickness, MathField, MathFraction};
use tex_state::node::KernKind;
use tex_state::scaled::Scaled;

use super::delimiters::make_delimiter;
use super::{
    Context, FrozenHList, MathBox, MathNode, MathTypesetState, add, boxed_node, clean_box, mul, sub,
};

pub(super) fn make_fraction(
    ctx: &mut Context<'_, impl MathTypesetState>,
    fraction: &MathFraction,
) -> FrozenHList {
    // AppG rule 15
    let size_params = ctx.params.for_size(ctx.style.size());
    let thickness = match fraction.thickness {
        FractionThickness::Default => size_params.extension.default_rule_thickness,
        FractionThickness::Explicit(value) => value,
    };
    let num_style = ctx.style.num_style();
    let denom_style = ctx.style.denom_style();
    let mut numerator = clean_box(ctx, &MathField::SubMlist(fraction.numerator), num_style);
    let mut denominator = clean_box(ctx, &MathField::SubMlist(fraction.denominator), denom_style);
    // AppG rule 15a
    if numerator.width < denominator.width {
        rebox(ctx, &mut numerator, denominator.width);
    } else {
        rebox(ctx, &mut denominator, numerator.width);
    }

    // AppG rule 15b
    let (mut shift_up, mut shift_down) = if ctx.style.is_display() {
        (size_params.symbols.num1, size_params.symbols.denom1)
    } else {
        (
            if thickness.raw() != 0 {
                size_params.symbols.num2
            } else {
                size_params.symbols.num3
            },
            size_params.symbols.denom2,
        )
    };

    if thickness.raw() == 0 {
        adjust_without_rule(
            &numerator,
            &denominator,
            ctx.style.is_display(),
            size_params.extension.default_rule_thickness,
            &mut shift_up,
            &mut shift_down,
        );
    } else {
        adjust_with_rule(
            &numerator,
            &denominator,
            thickness,
            size_params.symbols.axis_height,
            ctx.style.is_display(),
            &mut shift_up,
            &mut shift_down,
        );
    }

    let fraction_box = fraction_vlist(
        ctx,
        numerator,
        denominator,
        thickness,
        size_params.symbols.axis_height,
        shift_up,
        shift_down,
    );
    let target = if ctx.style.is_display() {
        size_params.symbols.delim1
    } else {
        size_params.symbols.delim2
    };
    // AppG rule 15e
    let left = make_delimiter(ctx, fraction.left_delimiter.unwrap_or(0), target);
    let right = make_delimiter(ctx, fraction.right_delimiter.unwrap_or(0), target);
    let list = ctx.layout.hlist([
        boxed_node(left),
        MathNode::VList(fraction_box),
        boxed_node(right),
    ]);
    let boxed = ctx.layout.hpack(list);
    ctx.layout.hlist([MathNode::HList(boxed)])
}

fn adjust_without_rule(
    numerator: &MathBox,
    denominator: &MathBox,
    display: bool,
    default_rule_thickness: Scaled,
    shift_up: &mut Scaled,
    shift_down: &mut Scaled,
) {
    // AppG rule 15c
    let multiplier: i32 = if display { 7 } else { 3 };
    let clearance = mul(multiplier, default_rule_thickness);
    let actual = sub(
        sub(*shift_up, numerator.depth),
        sub(denominator.height, *shift_down),
    );
    let delta = Scaled::from_raw(tex_arith::half(sub(clearance, actual).raw()));
    if delta.raw() > 0 {
        *shift_up = add(*shift_up, delta);
        *shift_down = add(*shift_down, delta);
    }
}

fn adjust_with_rule(
    numerator: &MathBox,
    denominator: &MathBox,
    thickness: Scaled,
    axis_height: Scaled,
    display: bool,
    shift_up: &mut Scaled,
    shift_down: &mut Scaled,
) {
    // AppG rule 15d
    let multiplier: i32 = if display { 3 } else { 1 };
    let clearance = mul(multiplier, thickness);
    let delta = Scaled::from_raw(tex_arith::half(thickness.raw()));
    let above_actual = sub(sub(*shift_up, numerator.depth), add(axis_height, delta));
    let below_actual = sub(
        sub(axis_height, delta),
        sub(denominator.height, *shift_down),
    );
    let delta1 = sub(clearance, above_actual);
    let delta2 = sub(clearance, below_actual);
    if delta1.raw() > 0 {
        *shift_up = add(*shift_up, delta1);
    }
    if delta2.raw() > 0 {
        *shift_down = add(*shift_down, delta2);
    }
}

fn fraction_vlist(
    ctx: &mut Context<'_, impl MathTypesetState>,
    numerator: MathBox,
    denominator: MathBox,
    thickness: Scaled,
    axis_height: Scaled,
    shift_up: Scaled,
    shift_down: Scaled,
) -> MathBox {
    let width = numerator.width;
    let height = add(shift_up, numerator.height);
    let depth = add(denominator.depth, shift_down);
    let numerator_depth = numerator.depth;
    let list = if thickness.raw() == 0 {
        ctx.layout.hlist([
            MathNode::HList(numerator),
            MathNode::Kern {
                amount: sub(
                    sub(shift_up, numerator_depth),
                    sub(denominator.height, shift_down),
                ),
                kind: KernKind::Explicit,
            },
            MathNode::HList(denominator),
        ])
    } else {
        let delta = Scaled::from_raw(tex_arith::half(thickness.raw()));
        ctx.layout.hlist([
            MathNode::HList(numerator),
            MathNode::Kern {
                amount: sub(sub(shift_up, numerator_depth), add(axis_height, delta)),
                kind: KernKind::Explicit,
            },
            MathNode::Rule {
                width: Some(width),
                height: Some(thickness),
                depth: Some(Scaled::from_raw(0)),
            },
            MathNode::Kern {
                amount: sub(sub(axis_height, delta), sub(denominator.height, shift_down)),
                kind: KernKind::Explicit,
            },
            MathNode::HList(denominator),
        ])
    };
    MathBox {
        width,
        height,
        depth,
        shift: Scaled::from_raw(0),
        list,
        axis: super::BoxAxis::Vertical,
        display: false,
        glue_set: tex_state::scaled::GlueSetRatio::from_raw(0),
        glue_sign: tex_state::node::Sign::Normal,
        glue_order: tex_state::glue::Order::Normal,
    }
}

fn rebox(ctx: &mut Context<'_, impl MathTypesetState>, boxed: &mut MathBox, width: Scaled) {
    let slack = sub(width, boxed.width);
    // TeX's rebox changes the width field directly when list_ptr(b)=null.
    // Materializing centering nodes in that case turns an empty box into a
    // nonempty one and can force an otherwise-dead DVI cursor movement.
    if slack.raw() != 0
        && !boxed.list.is_empty()
        && matches!(boxed.axis, super::BoxAxis::Horizontal)
    {
        let left = Scaled::from_raw(tex_arith::half(slack.raw()));
        let right = sub(slack, left);
        let left_node = (left.raw() != 0).then_some(MathNode::Kern {
            amount: left,
            kind: KernKind::Explicit,
        });
        let right_node = (right.raw() != 0).then_some(MathNode::Kern {
            amount: right,
            kind: KernKind::Explicit,
        });
        let nodes = left_node
            .into_iter()
            .chain([MathNode::Sequence(boxed.list)])
            .chain(right_node);
        boxed.list = ctx.layout.hlist(nodes);
    }
    boxed.width = width;
}
