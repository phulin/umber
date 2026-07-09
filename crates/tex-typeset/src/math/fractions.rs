use tex_state::math::{FractionThickness, MathField, MathFraction};
use tex_state::node::KernKind;
use tex_state::scaled::Scaled;

use super::delimiters::make_delimiter;
use super::{
    Context, FrozenHList, MathBox, MathNode, MathTypesetState, add, boxed_node, clean_box, hpack,
    sub,
};

pub(super) fn make_fraction(
    ctx: &Context<'_, impl MathTypesetState>,
    fraction: &MathFraction,
) -> FrozenHList {
    // AppG rule 15
    let size_params = ctx.params.for_size(ctx.style.size());
    let thickness = match fraction.thickness {
        FractionThickness::Default => size_params.extension.default_rule_thickness,
        FractionThickness::Explicit(value) => value,
    };
    let mut numerator = clean_box(
        ctx.state,
        &MathField::SubMlist(fraction.numerator),
        ctx.style.num_style(),
        ctx.params,
    );
    let mut denominator = clean_box(
        ctx.state,
        &MathField::SubMlist(fraction.denominator),
        ctx.style.denom_style(),
        ctx.params,
    );
    if numerator.width < denominator.width {
        rebox(&mut numerator, denominator.width);
    } else {
        rebox(&mut denominator, numerator.width);
    }

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
    let left = make_delimiter(ctx, fraction.left_delimiter.unwrap_or(0), target);
    let right = make_delimiter(ctx, fraction.right_delimiter.unwrap_or(0), target);
    let boxed = hpack(FrozenHList {
        nodes: vec![
            boxed_node(left),
            MathNode::VList(fraction_box),
            boxed_node(right),
        ],
    });
    FrozenHList {
        nodes: vec![MathNode::HList(boxed)],
    }
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
    let clearance = Scaled::from_raw(multiplier.saturating_mul(default_rule_thickness.raw()));
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
    let clearance = Scaled::from_raw(multiplier.saturating_mul(thickness.raw()));
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
    numerator: MathBox,
    denominator: MathBox,
    thickness: Scaled,
    axis_height: Scaled,
    shift_up: Scaled,
    shift_down: Scaled,
) -> MathBox {
    // AppG rule 15e
    let width = numerator.width;
    let height = add(shift_up, numerator.height);
    let depth = add(denominator.depth, shift_down);
    let nodes = if thickness.raw() == 0 {
        vec![
            MathNode::HList(numerator.clone()),
            MathNode::Kern {
                amount: sub(
                    sub(shift_up, numerator.depth),
                    sub(denominator.height, shift_down),
                ),
                kind: KernKind::Explicit,
            },
            MathNode::HList(denominator),
        ]
    } else {
        let delta = Scaled::from_raw(tex_arith::half(thickness.raw()));
        vec![
            MathNode::HList(numerator.clone()),
            MathNode::Kern {
                amount: sub(sub(shift_up, numerator.depth), add(axis_height, delta)),
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
        ]
    };
    MathBox {
        width,
        height,
        depth,
        shift: Scaled::from_raw(0),
        list: FrozenHList { nodes },
        axis: super::BoxAxis::Vertical,
    }
}

fn rebox(boxed: &mut MathBox, width: Scaled) {
    // AppG rule 15b
    let slack = sub(width, boxed.width);
    if slack.raw() != 0 && matches!(boxed.axis, super::BoxAxis::Horizontal) {
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
