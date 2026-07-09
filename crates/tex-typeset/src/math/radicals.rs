use tex_fonts::{LigKernChar, LigKernCommand};
use tex_state::math::{MathChar, MathField, MathNoad};
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

use super::delimiters::make_delimiter;
use super::{
    BoxAxis, Context, FetchedChar, FrozenHList, MathBox, MathNode, MathTypesetState, add,
    boxed_node, char_box, clean_box, fetch, hpack, make_character_nucleus, scripts, source_node,
    sub, vpack,
};

pub(super) struct AccentResult {
    pub hlist: FrozenHList,
    pub scripts_handled: bool,
}

pub(super) fn make_over(
    ctx: &Context<'_, impl MathTypesetState>,
    nucleus: &MathField,
) -> FrozenHList {
    // AppG rule 10
    let thickness = ctx
        .params
        .for_size(ctx.style.size())
        .extension
        .default_rule_thickness;
    let base = clean_box(ctx.state, nucleus, ctx.style.cramped_style(), ctx.params);
    FrozenHList {
        nodes: vec![boxed_node(overbar(
            base,
            Scaled::from_raw(3 * thickness.raw()),
            thickness,
        ))],
    }
}

pub(super) fn make_under(
    ctx: &Context<'_, impl MathTypesetState>,
    nucleus: &MathField,
) -> FrozenHList {
    // AppG rule 9
    let thickness = ctx
        .params
        .for_size(ctx.style.size())
        .extension
        .default_rule_thickness;
    let base = clean_box(ctx.state, nucleus, ctx.style, ctx.params);
    let mut under = vpack(FrozenHList {
        nodes: vec![
            MathNode::HList(base.clone()),
            MathNode::Kern {
                amount: Scaled::from_raw(3 * thickness.raw()),
                kind: KernKind::Explicit,
            },
            MathNode::Rule {
                width: Some(base.width),
                height: Some(thickness),
                depth: Some(Scaled::from_raw(0)),
            },
        ],
    });
    let delta = add(add(under.height, under.depth), thickness);
    under.height = base.height;
    under.depth = sub(delta, under.height);
    FrozenHList {
        nodes: vec![boxed_node(under)],
    }
}

pub(super) fn make_vcenter(
    ctx: &Context<'_, impl MathTypesetState>,
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
    FrozenHList {
        nodes: vec![boxed_node(centered)],
    }
}

fn clean_vcenter_box(ctx: &Context<'_, impl MathTypesetState>, nucleus: &MathField) -> MathBox {
    if let MathField::SubBox(list) = nucleus
        && let [Node::VList(boxed)] = ctx.state.nodes(*list)
    {
        return MathBox {
            width: boxed.width,
            height: boxed.height,
            depth: boxed.depth,
            shift: boxed.shift,
            list: FrozenHList {
                nodes: ctx
                    .state
                    .nodes(boxed.children)
                    .iter()
                    .map(|node| source_node(ctx.state, node))
                    .collect(),
            },
            axis: BoxAxis::Vertical,
        };
    }
    clean_box(ctx.state, nucleus, ctx.style, ctx.params)
}

pub(super) fn make_radical(
    ctx: &Context<'_, impl MathTypesetState>,
    noad: &MathNoad,
    delimiter: u32,
) -> FrozenHList {
    // AppG rule 11
    let size_params = ctx.params.for_size(ctx.style.size());
    let thickness = size_params.extension.default_rule_thickness;
    let x = clean_box(
        ctx.state,
        &noad.nucleus,
        ctx.style.cramped_style(),
        ctx.params,
    );
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
    let bar = overbar(x, clearance, delimiter.height);
    FrozenHList {
        nodes: vec![MathNode::HList(hpack(FrozenHList {
            nodes: vec![boxed_node(delimiter), boxed_node(bar)],
        }))],
    }
}

pub(super) fn make_math_accent(
    ctx: &Context<'_, impl MathTypesetState>,
    noad: &MathNoad,
    accent: MathChar,
) -> AccentResult {
    // AppG rule 12
    let Some(fetched) = fetch(ctx.state, accent, ctx.style) else {
        return AccentResult {
            hlist: FrozenHList::default(),
            scripts_handled: false,
        };
    };
    let accent_font = fetched.font;
    let mut accent_code = fetched.ch as u8;
    let mut accent_metrics = fetched.metrics;
    let skew = accent_skew(ctx, &noad.nucleus);
    let mut accentee = clean_box(
        ctx.state,
        &noad.nucleus,
        ctx.style.cramped_style(),
        ctx.params,
    );
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
            ctx.state,
            &mut base,
            &noad.subscript,
            &noad.superscript,
            ctx.style,
            ctx.params,
            script_delta,
        );
        accentee = hpack(base);
        delta = add(delta, sub(accentee.height, accentee_height));
        accentee_height = accentee.height;
        scripts_handled = true;
    }

    let mut accent_box = char_box(FetchedChar {
        font: accent_font,
        ch: char::from(accent_code),
        metrics: accent_metrics,
    });
    accent_box.shift = add(
        skew,
        Scaled::from_raw(tex_arith::half(sub(accentee_width, accent_box.width).raw())),
    );
    accent_box.width = Scaled::from_raw(0);

    let mut packed = vpack(FrozenHList {
        nodes: vec![
            MathNode::HList(accent_box),
            MathNode::Kern {
                amount: Scaled::from_raw(-delta.raw()),
                kind: KernKind::Accent,
            },
            MathNode::HList(accentee.clone()),
        ],
    });
    packed.width = accentee.width;
    if packed.height < accentee_height {
        packed.list.nodes.insert(
            0,
            MathNode::Kern {
                amount: sub(accentee_height, packed.height),
                kind: KernKind::Accent,
            },
        );
        packed.height = accentee_height;
    }
    AccentResult {
        hlist: FrozenHList {
            nodes: vec![MathNode::VList(packed)],
        },
        scripts_handled,
    }
}

fn overbar(base: MathBox, clearance: Scaled, thickness: Scaled) -> MathBox {
    let width = base.width;
    vpack(FrozenHList {
        nodes: vec![
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
            MathNode::HList(base),
        ],
    })
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
