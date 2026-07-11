use tex_arith::{half, saturating_add as add, saturating_sub as sub};
use tex_state::math::MathField;
use tex_state::node::KernKind;
use tex_state::scaled::Scaled;

use super::style::Style;
use super::{
    Context, FrozenHList, MathBox, MathNode, MathTypesetState, boxed_node, clean_box,
    hlist_extents, node_is_char,
};

pub fn make_scripts(
    ctx: &mut Context<'_, impl MathTypesetState>,
    base: &mut FrozenHList,
    subscript: &MathField,
    superscript: &MathField,
    style: Style,
    delta: Scaled,
) {
    // AppG rule 18a
    let size_params = ctx.params.for_size(style.size());
    let (mut shift_up, mut shift_down) = if ctx.layout.first_node(*base).is_some_and(node_is_char) {
        (Scaled::from_raw(0), Scaled::from_raw(0))
    } else {
        let (height, depth) = hlist_extents(*base);
        let t = if style.is_script_or_smaller() {
            tex_state::math::MathFontSize::ScriptScript
        } else {
            tex_state::math::MathFontSize::Script
        };
        let t_params = ctx.params.for_size(t).symbols;
        (
            sub(height, t_params.sup_drop),
            add(depth, t_params.sub_drop),
        )
    };

    let script = if matches!(superscript, MathField::Empty) {
        subscript_only(ctx, subscript, style, &mut shift_down)
    } else {
        let mut sup = superscript_box(ctx, superscript, style, &mut shift_up);
        if matches!(subscript, MathField::Empty) {
            sup.shift = shift_up;
            sup
        } else {
            let mut shifts = ScriptShifts {
                up: shift_up,
                down: shift_down,
            };
            script_pair(ctx, subscript, style, &mut shifts, delta, sup)
        }
    };
    *base = ctx
        .layout
        .hlist([MathNode::Sequence(*base), boxed_node(script)]);

    let _ = size_params;
}

fn subscript_only(
    ctx: &mut Context<'_, impl MathTypesetState>,
    subscript: &MathField,
    style: Style,
    shift_down: &mut Scaled,
) -> MathBox {
    // AppG rule 18b
    let size_params = ctx.params.for_size(style.size());
    let mut x = clean_box(ctx, subscript, style.sub_style());
    x.width = add(x.width, ctx.params.script_space);
    if *shift_down < size_params.symbols.sub1 {
        *shift_down = size_params.symbols.sub1;
    }
    let clr = sub(
        x.height,
        Scaled::from_raw((size_params.symbols.math_x_height.raw().abs() * 4) / 5),
    );
    if *shift_down < clr {
        *shift_down = clr;
    }
    x.shift = neg(*shift_down);
    x
}

fn superscript_box(
    ctx: &mut Context<'_, impl MathTypesetState>,
    superscript: &MathField,
    style: Style,
    shift_up: &mut Scaled,
) -> MathBox {
    // AppG rule 18c
    let size_params = ctx.params.for_size(style.size());
    let mut x = clean_box(ctx, superscript, style.sup_style());
    x.width = add(x.width, ctx.params.script_space);
    let clr = if style.cramped() {
        size_params.symbols.sup3
    } else if style.is_display() {
        size_params.symbols.sup1
    } else {
        size_params.symbols.sup2
    };
    if *shift_up < clr {
        *shift_up = clr;
    }
    let clr = add(
        x.depth,
        Scaled::from_raw(size_params.symbols.math_x_height.raw().abs() / 4),
    );
    if *shift_up < clr {
        *shift_up = clr;
    }
    x
}

fn script_pair(
    ctx: &mut Context<'_, impl MathTypesetState>,
    subscript: &MathField,
    style: Style,
    shifts: &mut ScriptShifts,
    delta: Scaled,
    mut sup: MathBox,
) -> MathBox {
    // AppG rule 18d
    let size_params = ctx.params.for_size(style.size());
    let mut sub_box = clean_box(ctx, subscript, style.sub_style());
    sub_box.width = add(sub_box.width, ctx.params.script_space);
    if shifts.down < size_params.symbols.sub2 {
        shifts.down = size_params.symbols.sub2;
    }
    // AppG rule 18e
    let gap = sub(sub(shifts.up, sup.depth), sub(sub_box.height, shifts.down));
    let clr = sub(
        Scaled::from_raw(4 * size_params.extension.default_rule_thickness.raw()),
        gap,
    );
    if clr.raw() > 0 {
        shifts.down = add(shifts.down, clr);
        let raised = sub(
            Scaled::from_raw((size_params.symbols.math_x_height.raw().abs() * 4) / 5),
            sub(shifts.up, sup.depth),
        );
        if raised.raw() > 0 {
            shifts.up = add(shifts.up, raised);
            shifts.down = sub(shifts.down, raised);
        }
    }
    // AppG rule 18f
    sup.shift = delta;
    let kern = sub(sub(shifts.up, sup.depth), sub(sub_box.height, shifts.down));
    let list = ctx.layout.hlist([
        MathNode::HList(sup),
        MathNode::Kern {
            amount: kern,
            kind: KernKind::Explicit,
        },
        MathNode::HList(sub_box),
    ]);
    let mut pair = ctx.layout.vpack(list);
    pair.shift = neg(shifts.down);
    pair
}

struct ScriptShifts {
    up: Scaled,
    down: Scaled,
}

fn neg(value: Scaled) -> Scaled {
    Scaled::from_raw(-value.raw())
}

#[allow(dead_code)]
fn tex_half(value: Scaled) -> Scaled {
    Scaled::from_raw(half(value.raw()))
}
