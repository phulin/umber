use tex_fonts::{LigKernChar, LigKernCommand};
use tex_state::math::{MathChar, MathField, MathNoad};
use tex_state::node::KernKind;
use tex_state::scaled::Scaled;

use super::delimiters::make_delimiter;
use super::{
    BoxAxis, Context, FetchedChar, FrozenHList, MathBox, MathNode, MathTypesetState, add,
    boxed_node, char_box, clean_box, fetch, make_character_nucleus, scripts, source_list, sub,
};

pub(super) struct AccentResult {
    pub hlist: FrozenHList,
    pub scripts_handled: bool,
}

pub(super) fn make_over(
    ctx: &mut Context<'_, impl MathTypesetState>,
    nucleus: &MathField,
) -> FrozenHList {
    // AppG rule 10
    let thickness = ctx
        .params
        .for_size(ctx.style.size())
        .extension
        .default_rule_thickness;
    let style = ctx.style.cramped_style();
    let base = clean_box(ctx, nucleus, style);
    let overbar = overbar(ctx, base, Scaled::from_raw(3 * thickness.raw()), thickness);
    ctx.layout.hlist([boxed_node(overbar)])
}

pub(super) fn make_under(
    ctx: &mut Context<'_, impl MathTypesetState>,
    nucleus: &MathField,
) -> FrozenHList {
    // AppG rule 9
    let thickness = ctx
        .params
        .for_size(ctx.style.size())
        .extension
        .default_rule_thickness;
    let base = clean_box(ctx, nucleus, ctx.style);
    let base_height = base.height;
    let base_width = base.width;
    let list = ctx.layout.hlist([
        boxed_node(base),
        MathNode::Kern {
            amount: Scaled::from_raw(3 * thickness.raw()),
            kind: KernKind::Explicit,
        },
        MathNode::Rule {
            width: Some(base_width),
            height: Some(thickness),
            depth: Some(Scaled::from_raw(0)),
        },
    ]);
    let mut under = ctx.layout.vpack(list);
    let delta = add(add(under.height, under.depth), thickness);
    under.height = base_height;
    under.depth = sub(delta, under.height);
    ctx.layout.hlist([boxed_node(under)])
}

pub(super) fn make_vcenter(
    ctx: &mut Context<'_, impl MathTypesetState>,
    nucleus: &MathField,
) -> FrozenHList {
    // AppG rule 18d
    let mut centered = clean_vcenter_box(ctx, nucleus);
    let delta = add(centered.height, centered.depth);
    centered.height = add(
        ctx.params.for_size(ctx.style.size()).symbols.axis_height,
        Scaled::from_raw(tex_arith::half(delta.raw())),
    );
    centered.depth = sub(delta, centered.height);
    ctx.layout.hlist([boxed_node(centered)])
}

fn clean_vcenter_box(ctx: &mut Context<'_, impl MathTypesetState>, nucleus: &MathField) -> MathBox {
    if let MathField::SubBox(list) = nucleus
        && let nodes = ctx.state.nodes(*list)
        && nodes.len() == 1
        && let Some(tex_state::node_arena::NodeRef::VList(boxed)) = nodes.first()
    {
        let list = source_list(ctx, boxed.children);
        return MathBox {
            width: boxed.width,
            height: boxed.height,
            depth: boxed.depth,
            shift: boxed.shift,
            list,
            axis: BoxAxis::Vertical,
            display: boxed.display,
            glue_set: boxed.glue_set,
            glue_sign: boxed.glue_sign,
            glue_order: boxed.glue_order,
        };
    }
    clean_box(ctx, nucleus, ctx.style)
}

fn unwrap_single_vlist(ctx: &Context<'_, impl MathTypesetState>, boxed: MathBox) -> MathBox {
    if boxed.axis == BoxAxis::Horizontal
        && boxed.shift.raw() == 0
        && let Some(MathNode::VList(inner)) = ctx.layout.single_node(boxed.list)
    {
        return inner.clone();
    }
    boxed
}

pub(super) fn make_radical(
    ctx: &mut Context<'_, impl MathTypesetState>,
    noad: &MathNoad,
    delimiter: u32,
) -> FrozenHList {
    // AppG rule 11
    let size_params = ctx.params.for_size(ctx.style.size());
    let thickness = size_params.extension.default_rule_thickness;
    let style = ctx.style.cramped_style();
    let x = clean_box(ctx, &noad.nucleus, style);
    let x = unwrap_single_vlist(ctx, x);
    let mut clearance = if ctx.style.is_display() {
        add(
            thickness,
            Scaled::from_raw(size_params.symbols.math_x_height.raw().abs() / 4),
        )
    } else {
        add(thickness, Scaled::from_raw(thickness.raw().abs() / 4))
    };
    let target = add(add(add(x.height, x.depth), clearance), thickness);
    let mut delimiter = make_delimiter(ctx, delimiter, target);
    let delta = sub(delimiter.depth, add(add(x.height, x.depth), clearance));
    if delta.raw() > 0 {
        clearance = add(clearance, Scaled::from_raw(tex_arith::half(delta.raw())));
    }
    delimiter.shift = add(x.height, clearance);
    let bar = overbar(ctx, x, clearance, delimiter.height);
    let inner = ctx.layout.hlist([boxed_node(delimiter), boxed_node(bar)]);
    let packed = ctx.layout.hpack(inner);
    ctx.layout.hlist([MathNode::HList(packed)])
}

pub(super) fn make_math_accent(
    ctx: &mut Context<'_, impl MathTypesetState>,
    noad: &MathNoad,
    accent: MathChar,
) -> AccentResult {
    // AppG rule 12
    let Some(fetched) = fetch(ctx.state, accent, ctx.style) else {
        return AccentResult {
            hlist: ctx.layout.empty(),
            scripts_handled: false,
        };
    };
    let accent_font = fetched.font;
    let mut accent_code = fetched.ch as u8;
    let mut accent_metrics = fetched.metrics;
    let skew = accent_skew(ctx, &noad.nucleus);
    let style = ctx.style.cramped_style();
    let mut accentee = clean_box(ctx, &noad.nucleus, style);
    let accentee_width = accentee.width;
    let mut accentee_height = accentee.height;

    while let Some(next) = ctx.state.font_next_larger(accent_font, accent_code) {
        let Some(next_metrics) = ctx.state.font_char_metrics(accent_font, next) else {
            break;
        };
        if next_metrics.width > accentee_width {
            break;
        }
        accent_code = next;
        accent_metrics = next_metrics;
    }

    let x_height = ctx.state.font_parameter(accent_font, 5);
    let mut delta = if accentee_height < x_height {
        accentee_height
    } else {
        x_height
    };
    let mut scripts_handled = false;
    if (!matches!(noad.subscript, MathField::Empty)
        || !matches!(noad.superscript, MathField::Empty))
        && let MathField::MathChar(ch) = noad.nucleus
    {
        let mut script_delta = Scaled::from_raw(0);
        let mut base = make_character_nucleus(ctx, ch, false, &noad.subscript, &mut script_delta);
        scripts::make_scripts(
            ctx,
            &mut base,
            &noad.subscript,
            &noad.superscript,
            ctx.style,
            script_delta,
        );
        accentee = ctx.layout.hpack(base);
        delta = add(delta, sub(accentee.height, accentee_height));
        accentee_height = accentee.height;
        scripts_handled = true;
    }

    let mut accent_box = char_box(
        ctx,
        FetchedChar {
            font: accent_font,
            ch: char::from(accent_code),
            metrics: accent_metrics,
        },
    );
    accent_box.shift = add(
        skew,
        Scaled::from_raw(tex_arith::half(sub(accentee_width, accent_box.width).raw())),
    );
    accent_box.width = Scaled::from_raw(0);

    let accentee_width = accentee.width;
    let list = ctx.layout.hlist([
        MathNode::HList(accent_box),
        MathNode::Kern {
            amount: Scaled::from_raw(-delta.raw()),
            kind: KernKind::Accent,
        },
        MathNode::HList(accentee),
    ]);
    let mut packed = ctx.layout.vpack(list);
    packed.width = accentee_width;
    if packed.height < accentee_height {
        packed.list = ctx.layout.hlist([
            MathNode::Kern {
                amount: sub(accentee_height, packed.height),
                kind: KernKind::Accent,
            },
            MathNode::Sequence(packed.list),
        ]);
        packed.height = accentee_height;
    }
    AccentResult {
        hlist: ctx.layout.hlist([MathNode::VList(packed)]),
        scripts_handled,
    }
}

fn overbar(
    ctx: &mut Context<'_, impl MathTypesetState>,
    base: MathBox,
    clearance: Scaled,
    thickness: Scaled,
) -> MathBox {
    let width = base.width;
    let list = ctx.layout.hlist([
        MathNode::Kern {
            amount: thickness,
            kind: KernKind::Explicit,
        },
        MathNode::Rule {
            width: Some(width),
            height: Some(thickness),
            depth: Some(Scaled::from_raw(0)),
        },
        MathNode::Kern {
            amount: clearance,
            kind: KernKind::Explicit,
        },
        boxed_node(base),
    ]);
    ctx.layout.vpack(list)
}

fn accent_skew(ctx: &Context<'_, impl MathTypesetState>, nucleus: &MathField) -> Scaled {
    let MathField::MathChar(ch) = nucleus else {
        return Scaled::from_raw(0);
    };
    let Some(fetched) = fetch(ctx.state, *ch, ctx.style) else {
        return Scaled::from_raw(0);
    };
    let Ok(left) = u8::try_from(u32::from(fetched.ch)) else {
        return Scaled::from_raw(0);
    };
    let skew = ctx.state.font_skew_char(fetched.font);
    let Ok(right) = u8::try_from(skew) else {
        return Scaled::from_raw(0);
    };
    match ctx.state.lig_kern_command(
        fetched.font,
        LigKernChar::Char(left),
        LigKernChar::Char(right),
    ) {
        Some(LigKernCommand::Kern(amount)) => amount,
        _ => Scaled::from_raw(0),
    }
}
