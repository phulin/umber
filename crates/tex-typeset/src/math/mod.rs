//! Pure Appendix G math-list to horizontal-list conversion.

mod model;
mod params;
mod scripts;
mod spacing;
mod style;

use tex_arith::{half, x_over_n};
use tex_fonts::CharMetrics;
use tex_state::Universe;
use tex_state::env::banks::DimenParam;
use tex_state::ids::{FontId, GlueId, NodeListId};
use tex_state::math::{
    LimitType, MathChar, MathField, MathFontSize, MathNoad, NoadClass, NoadKind,
};
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

use crate::TypesetState;

pub use model::{BoxAxis, FrozenHList, MathBox, MathGlueKind, MathNode};
pub(crate) use model::{hpack, node_is_char, vpack};
pub use params::{ExtensionParams, MathParams, SizeParams, SymbolParams};
pub use spacing::{SpacingKind, inter_noad_spacing, math_glue, math_kern};
pub use style::{Style, StyleFamily};

/// Immutable state access needed by the math typesetting kernel.
pub trait MathTypesetState: TypesetState {
    fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId;
    fn font_parameter(&self, font: FontId, number: u16) -> Scaled;
    fn dimen_param(&self, param: DimenParam) -> Scaled;
    fn muskip(&self, index: u16) -> GlueId;
}

#[derive(Clone, Debug)]
struct WorkNoad {
    class: NoadClass,
    hlist: FrozenHList,
    penalty: i32,
}

#[derive(Clone, Debug)]
enum WorkItem {
    Noad(WorkNoad),
    Node(MathNode),
    Style(Style),
}

#[derive(Clone, Copy, Debug)]
struct FetchedChar {
    font: FontId,
    ch: char,
    metrics: CharMetrics,
}

const INF_PENALTY: i32 = 10_000;
const BIN_OP_PENALTY: i32 = 700;
const REL_PENALTY: i32 = 500;

#[must_use]
pub fn mlist_to_hlist(
    state: &impl MathTypesetState,
    input: NodeListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
) -> FrozenHList {
    // AppG rule 18
    let mut ctx = Context {
        state,
        params,
        style,
        mu: math_unit(params, style),
    };
    let mut work = Vec::new();
    let mut max_height = Scaled::from_raw(0);
    let mut max_depth = Scaled::from_raw(0);
    first_pass(
        &mut ctx,
        state.nodes(input),
        &mut work,
        &mut max_height,
        &mut max_depth,
    );
    convert_final_bin_to_ord(&mut work);
    second_pass(&mut ctx, &work, penalties)
}

fn first_pass<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    input: &[Node],
    out: &mut Vec<WorkItem>,
    max_height: &mut Scaled,
    max_depth: &mut Scaled,
) {
    let mut r_type = Some(NoadClass::Op);
    let mut index = 0;
    while index < input.len() {
        match &input[index] {
            Node::MathStyle(style) => {
                // AppG rule 18
                ctx.set_style(Style::from_math_style(*style));
                out.push(WorkItem::Style(ctx.style));
            }
            Node::MathChoice(choice) => {
                // AppG rule 18
                out.push(WorkItem::Style(ctx.style));
                let selected = match ctx.style.family() {
                    StyleFamily::Display => choice.display,
                    StyleFamily::Text => choice.text,
                    StyleFamily::Script => choice.script,
                    StyleFamily::ScriptScript => choice.script_script,
                };
                first_pass(ctx, ctx.state.nodes(selected), out, max_height, max_depth);
            }
            Node::Glue { spec, kind } => {
                // AppG rule 18
                if matches!(kind, tex_state::node::GlueKind::NonScript)
                    && ctx.style.is_script_or_smaller()
                    && input
                        .get(index + 1)
                        .is_some_and(|next| matches!(next, Node::Glue { .. } | Node::Kern { .. }))
                {
                    index += 1;
                }
                let spec = if matches!(kind, tex_state::node::GlueKind::MuSkip) {
                    spacing::math_glue(ctx.state.glue(*spec), ctx.mu)
                } else {
                    ctx.state.glue(*spec)
                };
                out.push(WorkItem::Node(MathNode::Glue {
                    spec,
                    kind: if matches!(kind, tex_state::node::GlueKind::MuSkip) {
                        MathGlueKind::MuSkip
                    } else if matches!(kind, tex_state::node::GlueKind::NonScript) {
                        MathGlueKind::NonScript
                    } else {
                        MathGlueKind::Source
                    },
                }));
            }
            Node::Kern { amount, kind } => {
                // AppG rule 18
                out.push(WorkItem::Node(MathNode::Kern {
                    amount: if matches!(kind, KernKind::Mu) {
                        spacing::math_kern(*amount, ctx.mu)
                    } else {
                        *amount
                    },
                    kind: if matches!(kind, KernKind::Mu) {
                        KernKind::Explicit
                    } else {
                        *kind
                    },
                }));
            }
            Node::MathNoad(noad) => {
                // AppG rule 18
                let mut class = noad_class(noad);
                if class == NoadClass::Bin
                    && matches!(
                        r_type,
                        Some(
                            NoadClass::Bin
                                | NoadClass::Op
                                | NoadClass::Rel
                                | NoadClass::Open
                                | NoadClass::Punct
                        )
                    )
                {
                    class = NoadClass::Ord;
                }
                if matches!(class, NoadClass::Rel | NoadClass::Close | NoadClass::Punct) {
                    convert_final_bin_to_ord(out);
                }
                let work = translate_noad(ctx, noad, class);
                let packed = hpack(work.hlist.clone());
                *max_height = (*max_height).max(packed.height);
                *max_depth = (*max_depth).max(packed.depth);
                r_type = Some(work.class);
                out.push(WorkItem::Noad(work));
            }
            other => out.push(WorkItem::Node(source_node(ctx.state, other))),
        }
        index += 1;
    }
}

fn translate_noad<S: MathTypesetState>(
    ctx: &Context<'_, S>,
    noad: &MathNoad,
    class: NoadClass,
) -> WorkNoad {
    let mut delta = Scaled::from_raw(0);
    let mut hlist = match (&noad.kind, &noad.nucleus) {
        (NoadKind::Operator(limit), MathField::MathChar(ch))
            if !matches!(limit, LimitType::Limits) =>
        {
            make_operator(ctx, *ch, &noad.subscript, &mut delta)
        }
        (_, MathField::MathChar(ch) | MathField::MathTextChar(ch)) => make_character_nucleus(
            ctx,
            *ch,
            matches!(noad.nucleus, MathField::MathTextChar(_)),
            &noad.subscript,
            &mut delta,
        ),
        _ => FrozenHList {
            nodes: vec![MathNode::HList(clean_box(
                ctx.state,
                &noad.nucleus,
                ctx.style,
                ctx.params,
            ))],
        },
    };

    if !matches!(noad.subscript, MathField::Empty) || !matches!(noad.superscript, MathField::Empty)
    {
        scripts::make_scripts(
            ctx.state,
            &mut hlist,
            &noad.subscript,
            &noad.superscript,
            ctx.style,
            ctx.params,
            delta,
        );
    }
    WorkNoad {
        class,
        hlist,
        penalty: match class {
            NoadClass::Bin => BIN_OP_PENALTY,
            NoadClass::Rel => REL_PENALTY,
            _ => INF_PENALTY,
        },
    }
}

fn second_pass<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    work: &[WorkItem],
    penalties: bool,
) -> FrozenHList {
    // AppG rule 18
    let mut output = FrozenHList::default();
    let mut previous = None;
    for (index, item) in work.iter().enumerate() {
        match item {
            WorkItem::Style(style) => ctx.set_style(*style),
            WorkItem::Node(node) => output.nodes.push(node.clone()),
            WorkItem::Noad(noad) => {
                if let Some(left) = previous
                    && let Some(spec) = spacing::spacing_glue(
                        spacing::inter_noad_spacing(left, noad.class, ctx.style),
                        ctx.params,
                        ctx.mu,
                    )
                {
                    output.nodes.push(MathNode::Glue {
                        spec,
                        kind: MathGlueKind::MuSkip,
                    });
                }
                output.nodes.extend(noad.hlist.nodes.iter().cloned());
                if penalties
                    && noad.penalty < INF_PENALTY
                    && work.get(index + 1).is_some_and(|next| {
                        !matches!(next, WorkItem::Node(MathNode::Penalty(_)))
                            && !matches!(
                                next,
                                WorkItem::Noad(WorkNoad {
                                    class: NoadClass::Rel,
                                    ..
                                })
                            )
                    })
                {
                    output.nodes.push(MathNode::Penalty(noad.penalty));
                }
                previous = Some(noad.class);
            }
        }
    }
    output
}

pub(crate) fn clean_box(
    state: &impl MathTypesetState,
    field: &MathField,
    style: Style,
    params: &MathParams,
) -> MathBox {
    // AppG rule 17
    match field {
        MathField::Empty => hpack(FrozenHList::default()),
        MathField::MathChar(ch) | MathField::MathTextChar(ch) => {
            if let Some(fetched) = fetch(state, *ch, style) {
                char_box(fetched)
            } else {
                hpack(FrozenHList::default())
            }
        }
        MathField::SubBox(list) => hpack(FrozenHList {
            nodes: state
                .nodes(*list)
                .iter()
                .map(|node| source_node(state, node))
                .collect(),
        }),
        MathField::SubMlist(list) => hpack(mlist_to_hlist(state, *list, style, false, params)),
    }
}

fn make_character_nucleus<S: MathTypesetState>(
    ctx: &Context<'_, S>,
    ch: MathChar,
    text_char: bool,
    subscript: &MathField,
    delta: &mut Scaled,
) -> FrozenHList {
    // AppG rule 18
    let Some(fetched) = fetch(ctx.state, ch, ctx.style) else {
        return FrozenHList::default();
    };
    *delta = fetched.metrics.italic_correction;
    if text_char && ctx.state.font_parameter(fetched.font, 2).raw() != 0 {
        *delta = Scaled::from_raw(0);
    }
    let mut nodes = vec![MathNode::Char {
        font: fetched.font,
        ch: fetched.ch,
        metrics: fetched.metrics,
    }];
    if matches!(subscript, MathField::Empty) && delta.raw() != 0 {
        nodes.push(MathNode::Kern {
            amount: *delta,
            kind: KernKind::Font,
        });
        *delta = Scaled::from_raw(0);
    }
    FrozenHList { nodes }
}

fn make_operator<S: MathTypesetState>(
    ctx: &Context<'_, S>,
    ch: MathChar,
    subscript: &MathField,
    delta: &mut Scaled,
) -> FrozenHList {
    // AppG rule 13
    let Some(fetched) = fetch(ctx.state, ch, ctx.style) else {
        return FrozenHList::default();
    };
    *delta = fetched.metrics.italic_correction;
    let mut boxed = char_box(fetched);
    if !matches!(subscript, MathField::Empty) {
        boxed.width = sub(boxed.width, *delta);
    }
    let size_params = ctx.params.for_size(ctx.style.size());
    boxed.shift = sub(
        Scaled::from_raw(half(sub(boxed.height, boxed.depth).raw())),
        size_params.symbols.axis_height,
    );
    FrozenHList {
        nodes: vec![MathNode::HList(boxed)],
    }
}

fn char_box(fetched: FetchedChar) -> MathBox {
    // AppG rule 17
    let list = FrozenHList {
        nodes: vec![MathNode::Char {
            font: fetched.font,
            ch: fetched.ch,
            metrics: fetched.metrics,
        }],
    };
    MathBox {
        width: add(fetched.metrics.width, fetched.metrics.italic_correction),
        height: fetched.metrics.height,
        depth: fetched.metrics.depth,
        shift: Scaled::from_raw(0),
        list,
        axis: BoxAxis::Horizontal,
    }
}

fn fetch(state: &impl MathTypesetState, ch: MathChar, style: Style) -> Option<FetchedChar> {
    // AppG rule 17
    let code = u8::try_from(u32::from(ch.character)).ok()?;
    let font = state.math_family_font(style.size(), ch.family);
    let metrics = state.font_char_metrics(font, code)?;
    Some(FetchedChar {
        font,
        ch: ch.character,
        metrics,
    })
}

fn source_node(state: &impl MathTypesetState, node: &Node) -> MathNode {
    match node {
        Node::Char { font, ch } => {
            let code = u8::try_from(u32::from(*ch)).ok();
            if let Some(metrics) = code.and_then(|code| state.font_char_metrics(*font, code)) {
                MathNode::Char {
                    font: *font,
                    ch: *ch,
                    metrics,
                }
            } else {
                MathNode::Opaque(node.clone())
            }
        }
        Node::Kern { amount, kind } => MathNode::Kern {
            amount: *amount,
            kind: *kind,
        },
        Node::Glue { spec, .. } => MathNode::Glue {
            spec: state.glue(*spec),
            kind: MathGlueKind::Source,
        },
        Node::Penalty(penalty) => MathNode::Penalty(*penalty),
        Node::Rule {
            width,
            height,
            depth,
        } => MathNode::Rule {
            width: *width,
            height: *height,
            depth: *depth,
        },
        Node::HList(box_node) => MathNode::Opaque(Node::HList(box_node.clone())),
        Node::VList(box_node) => MathNode::Opaque(Node::VList(box_node.clone())),
        _ => MathNode::Opaque(node.clone()),
    }
}

fn noad_class(noad: &MathNoad) -> NoadClass {
    match noad.kind {
        NoadKind::Normal(class) => class,
        NoadKind::Operator(_) => NoadClass::Op,
        NoadKind::Radical { .. }
        | NoadKind::Accent { .. }
        | NoadKind::Underline
        | NoadKind::Overline
        | NoadKind::VCenter => NoadClass::Ord,
    }
}

fn convert_final_bin_to_ord(work: &mut [WorkItem]) {
    if let Some(WorkItem::Noad(noad)) = work
        .iter_mut()
        .rev()
        .find(|item| matches!(item, WorkItem::Noad(_)))
        && noad.class == NoadClass::Bin
    {
        // AppG rule 18
        noad.class = NoadClass::Ord;
        noad.penalty = INF_PENALTY;
    }
}

fn math_unit(params: &MathParams, style: Style) -> Scaled {
    // AppG rule 17
    x_over_n(params.for_size(style.size()).symbols.math_quad, 18)
        .expect("math quad divided by 18 has nonzero denominator")
        .quotient
}

fn add(left: Scaled, right: Scaled) -> Scaled {
    Scaled::from_raw(left.raw().saturating_add(right.raw()))
}

fn sub(left: Scaled, right: Scaled) -> Scaled {
    Scaled::from_raw(left.raw().saturating_sub(right.raw()))
}

struct Context<'a, S> {
    state: &'a S,
    params: &'a MathParams,
    style: Style,
    mu: Scaled,
}

impl<S> Context<'_, S> {
    fn set_style(&mut self, style: Style) {
        self.style = style;
        self.mu = math_unit(self.params, style);
    }
}

impl MathTypesetState for Universe {
    fn math_family_font(&self, size: MathFontSize, family: u8) -> FontId {
        Universe::math_family_font(self, size, family)
    }

    fn font_parameter(&self, font: FontId, number: u16) -> Scaled {
        Universe::font_parameter(self, font, number)
    }

    fn dimen_param(&self, param: DimenParam) -> Scaled {
        Universe::dimen_param(self, param)
    }

    fn muskip(&self, index: u16) -> GlueId {
        Universe::muskip(self, index)
    }
}

#[cfg(test)]
mod tests;
