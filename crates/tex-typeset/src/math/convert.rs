use tex_arith::x_over_n;
use tex_fonts::CharMetrics;
use tex_state::ids::{FontId, NodeListId};
use tex_state::math::{LimitType, MathChar, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;

use super::{
    BoxAxis, FrozenHList, MathBox, MathGlueKind, MathLayout, MathLayoutBuilder, MathLayoutSink,
    MathNode, MathParams, MathTypesetState, SpacingKind, Style, StyleFamily, add, boxed_node,
    delimiters, fractions, left_right_delimiter_target, operators, radicals, scripts, spacing,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct FetchedChar {
    pub(crate) font: FontId,
    pub(crate) ch: char,
    pub(crate) metrics: CharMetrics,
}

const INF_PENALTY: i32 = 10_000;

#[must_use]
pub fn mlist_to_hlist(
    state: &impl MathTypesetState,
    input: NodeListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
) -> MathLayout {
    build_math_layout(state, input, style, penalties, params)
}

#[must_use]
pub fn mlist_to_hlist_with_sink(
    state: &mut impl MathLayoutSink,
    input: NodeListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
) -> MathLayout {
    let layout = build_math_layout(&*state, input, style, penalties, params);
    state.finish_math_hlist(layout.root(), &layout);
    layout
}

fn build_math_layout(
    state: &impl MathTypesetState,
    input: NodeListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
) -> MathLayout {
    let mut ctx = Context {
        state,
        params,
        style,
        mu: math_unit(params, style),
        layout: MathLayoutBuilder::new(),
        converted: HashMap::new(),
    };
    prepare_nested_mlists(&mut ctx, input);
    let root = convert_mlist_uncached(&mut ctx, input, style, penalties);
    ctx.layout.finish(root)
}

pub(super) fn convert_mlist<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    input: NodeListId,
    style: Style,
    _penalties: bool,
) -> FrozenHList {
    *ctx.converted
        .get(&(input, style))
        .expect("nested math list was not prepared by the iterative conversion planner")
}

fn convert_mlist_uncached<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    input: NodeListId,
    style: Style,
    penalties: bool,
) -> FrozenHList {
    let saved_style = ctx.style;
    ctx.set_style(style);
    let input = expand_math_choices(ctx.state, input, style);
    let mut work = Vec::with_capacity(input.nodes.len());
    let mut max_height = Scaled::from_raw(0);
    let mut max_depth = Scaled::from_raw(0);
    first_pass(ctx, &input, &mut work, &mut max_height, &mut max_depth);
    convert_final_bin_to_ord(&mut work);
    let result = second_pass(ctx, style, work, penalties, max_height, max_depth);
    ctx.set_style(saved_style);
    result
}

#[derive(Clone, Debug)]
struct WorkNoad {
    class: NoadClass,
    hlist: FrozenHList,
    penalty: i32,
}

#[derive(Clone, Debug)]
struct WorkDelimiter {
    class: NoadClass,
    delimiter: u32,
}

#[derive(Clone, Debug)]
enum WorkItem {
    Noad(WorkNoad),
    Delimiter(WorkDelimiter),
    Node(MathNode),
    Style(Style),
}

fn first_pass<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    view: &ExpandedMathView,
    out: &mut Vec<WorkItem>,
    max_height: &mut Scaled,
    max_depth: &mut Scaled,
) {
    let original = view.nodes.as_slice();
    let marker_styles = view.marker_styles.as_slice();
    let mut rewritten = None::<Vec<Node>>;
    let mut style_marker = 0;
    let mut r_type = Some(NoadClass::Op);
    let mut index = 0;
    while index < rewritten.as_deref().unwrap_or(original).len() {
        if matches!(
            rewritten.as_deref().unwrap_or(original).get(index),
            Some(Node::MathNoad(MathNoad {
                kind: NoadKind::Normal(NoadClass::Bin),
                ..
            }))
        ) && matches!(
            r_type,
            Some(
                NoadClass::Bin
                    | NoadClass::Op
                    | NoadClass::Rel
                    | NoadClass::Open
                    | NoadClass::Punct
            )
        ) {
            let input = rewritten.get_or_insert_with(|| original.to_vec());
            if let Node::MathNoad(noad) = &mut input[index] {
                noad.kind = NoadKind::Normal(NoadClass::Ord);
            }
        }
        let input = rewritten.as_deref().unwrap_or(original);
        if matches!(
            &input[index],
            Node::MathNoad(noad) if matches!(noad.kind, NoadKind::Normal(NoadClass::Ord))
        ) && operators::ord_pair_may_change(input, index)
        {
            let input = rewritten.get_or_insert_with(|| original.to_vec());
            operators::make_ord(ctx, input, index);
        }
        let input = rewritten.as_deref().unwrap_or(original);
        match &input[index] {
            Node::MathStyle(style) => {
                // AppG rule 3
                let full_style = marker_styles
                    .get(style_marker)
                    .copied()
                    .unwrap_or_else(|| Style::from_math_style(*style));
                style_marker += 1;
                ctx.set_style(full_style);
                out.push(WorkItem::Style(ctx.style));
            }
            Node::MathChoice(_) => unreachable!("math choices are expanded by the iterative view"),
            Node::Glue { spec, kind, leader } => {
                // AppG rule 2
                if matches!(kind, GlueKind::NonScript)
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
                    kind: *kind,
                    leader: *leader,
                }));
            }
            Node::Kern { amount, kind } => {
                // AppG rule 2
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
            Node::MathNoad(noad)
                if matches!(
                    noad.kind,
                    NoadKind::LeftDelimiter { .. }
                        | NoadKind::RightDelimiter { .. }
                        | NoadKind::MiddleDelimiter { .. }
                ) =>
            {
                let (class, delimiter) = match noad.kind {
                    NoadKind::LeftDelimiter { delimiter } => (NoadClass::Open, delimiter),
                    NoadKind::RightDelimiter { delimiter } => (NoadClass::Close, delimiter),
                    NoadKind::MiddleDelimiter { delimiter } => (NoadClass::Rel, delimiter),
                    _ => unreachable!("guard restricts delimiter noads"),
                };
                if matches!(class, NoadClass::Close) {
                    // AppG rule 6
                    convert_final_bin_to_ord(out);
                }
                r_type = Some(class);
                out.push(WorkItem::Delimiter(WorkDelimiter { class, delimiter }));
            }
            Node::MathNoad(noad) => {
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
                    // AppG rule 5
                    class = NoadClass::Ord;
                }
                if matches!(class, NoadClass::Rel | NoadClass::Close | NoadClass::Punct) {
                    // AppG rule 6
                    convert_final_bin_to_ord(out);
                }
                // AppG rule 7: Open and Inner atoms fall through unchanged to Rule 17.
                let work = translate_noad(ctx, noad, class);
                let (height, depth) = super::hlist_extents(work.hlist);
                *max_height = (*max_height).max(height);
                *max_depth = (*max_depth).max(depth);
                r_type = Some(work.class);
                out.push(WorkItem::Noad(work));
            }
            Node::FractionNoad(fraction) => {
                // AppG rule 15
                let hlist = fractions::make_fraction(ctx, fraction);
                let (height, depth) = super::hlist_extents(hlist);
                *max_height = (*max_height).max(height);
                *max_depth = (*max_depth).max(depth);
                r_type = Some(NoadClass::Ord);
                out.push(WorkItem::Noad(WorkNoad {
                    class: NoadClass::Ord,
                    hlist,
                    penalty: INF_PENALTY,
                }));
            }
            other => {
                // AppG rule 1
                out.push(WorkItem::Node(source_node(ctx, other)));
            }
        }
        index += 1;
    }
}

/// Builds the immutable node view selected by Appendix G rule 4 without
/// recursively descending through nested `\mathchoice` lists.
struct ExpandedMathView {
    nodes: Vec<Node>,
    marker_styles: Vec<Style>,
}

fn expand_math_choices(
    state: &impl MathTypesetState,
    root: NodeListId,
    starting_style: Style,
) -> ExpandedMathView {
    #[derive(Clone, Copy)]
    struct Frame {
        list: NodeListId,
        index: usize,
    }

    let mut style = starting_style;
    let mut out = Vec::new();
    let mut marker_styles = Vec::new();
    let mut stack = vec![Frame {
        list: root,
        index: 0,
    }];
    while let Some(frame) = stack.last_mut() {
        let nodes = state.nodes(frame.list);
        let Some(node) = nodes.get(frame.index).map(|node| node.to_owned()) else {
            stack.pop();
            continue;
        };
        frame.index += 1;
        match node {
            Node::MathStyle(next) => {
                style = Style::from_math_style(next);
                out.push(Node::MathStyle(next));
                marker_styles.push(style);
            }
            Node::MathChoice(choice) => {
                // The style marker is semantically observable by the first
                // pass even though the choice itself disappears.
                out.push(Node::MathStyle(match style.family() {
                    StyleFamily::Display => tex_state::math::MathStyle::Display,
                    StyleFamily::Text => tex_state::math::MathStyle::Text,
                    StyleFamily::Script => tex_state::math::MathStyle::Script,
                    StyleFamily::ScriptScript => tex_state::math::MathStyle::ScriptScript,
                }));
                marker_styles.push(style);
                let selected = match style.family() {
                    StyleFamily::Display => choice.display,
                    StyleFamily::Text => choice.text,
                    StyleFamily::Script => choice.script,
                    StyleFamily::ScriptScript => choice.script_script,
                };
                stack.push(Frame {
                    list: selected,
                    index: 0,
                });
            }
            node => out.push(node),
        }
    }
    ExpandedMathView {
        nodes: out,
        marker_styles,
    }
}

const ALL_STYLES: [Style; 8] = [
    Style::DISPLAY,
    Style::new(StyleFamily::Display, true),
    Style::TEXT,
    Style::new(StyleFamily::Text, true),
    Style::SCRIPT,
    Style::new(StyleFamily::Script, true),
    Style::SCRIPT_SCRIPT,
    Style::new(StyleFamily::ScriptScript, true),
];

/// Converts structural sub-mlists bottom-up so Appendix G conversion never
/// follows a source-list edge on the Rust call stack. Math-choice branches are
/// scanned as inline views, matching rule 4, rather than converted separately.
fn prepare_nested_mlists<S: MathTypesetState>(ctx: &mut Context<'_, S>, root: NodeListId) {
    let mut visiting = HashSet::new();
    let mut completed = HashSet::new();
    let mut stack = vec![(root, false)];
    let mut postorder = Vec::new();
    while let Some((list, expanded)) = stack.pop() {
        if expanded {
            visiting.remove(&list);
            completed.insert(list);
            postorder.push(list);
            continue;
        }
        if completed.contains(&list) {
            continue;
        }
        assert!(
            visiting.insert(list),
            "math source lists must not contain structural cycles"
        );
        stack.push((list, true));
        for dependency in direct_submlists(ctx.state, list) {
            stack.push((dependency, false));
        }
    }

    for list in postorder.into_iter().filter(|list| *list != root) {
        for style in ALL_STYLES {
            let converted = convert_mlist_uncached(ctx, list, style, false);
            ctx.converted.insert((list, style), converted);
        }
    }
}

fn direct_submlists(state: &impl MathTypesetState, root: NodeListId) -> Vec<NodeListId> {
    fn add_field(field: &MathField, out: &mut HashSet<NodeListId>) {
        if let MathField::SubMlist(list) = field {
            out.insert(*list);
        }
    }

    let mut out = HashSet::new();
    let mut choice_lists = vec![root];
    let mut visited = HashSet::new();
    while let Some(list) = choice_lists.pop() {
        if !visited.insert(list) {
            continue;
        }
        for node in state.nodes(list) {
            match node.to_owned() {
                Node::MathNoad(noad) => {
                    add_field(&noad.nucleus, &mut out);
                    add_field(&noad.subscript, &mut out);
                    add_field(&noad.superscript, &mut out);
                }
                Node::FractionNoad(fraction) => {
                    out.insert(fraction.numerator);
                    out.insert(fraction.denominator);
                }
                Node::MathChoice(choice) => choice_lists.extend([
                    choice.display,
                    choice.text,
                    choice.script,
                    choice.script_script,
                ]),
                _ => {}
            }
        }
    }
    out.into_iter().collect()
}

fn translate_noad<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    noad: &MathNoad,
    class: NoadClass,
) -> WorkNoad {
    let mut delta = Scaled::from_raw(0);
    let mut scripts_handled = false;
    let mut hlist = match (&noad.kind, &noad.nucleus) {
        (NoadKind::Operator(limit), _) => {
            let result = operators::make_op(ctx, noad, *limit);
            delta = result.delta;
            scripts_handled = result.scripts_handled;
            result.hlist
        }
        (NoadKind::Normal(NoadClass::Op), _) => {
            // A class-1 \mathchar is an op_noad with TeX's normal subtype,
            // which means limits in display style and side scripts otherwise.
            let result = operators::make_op(ctx, noad, LimitType::DisplayLimits);
            delta = result.delta;
            scripts_handled = result.scripts_handled;
            result.hlist
        }
        (NoadKind::Radical { delimiter }, _) => radicals::make_radical(ctx, noad, *delimiter),
        (NoadKind::Accent { accent }, _) => {
            let result = radicals::make_math_accent(ctx, noad, *accent);
            scripts_handled = result.scripts_handled;
            result.hlist
        }
        (NoadKind::Underline, _) => radicals::make_under(ctx, &noad.nucleus),
        (NoadKind::Overline, _) => radicals::make_over(ctx, &noad.nucleus),
        (NoadKind::VCenter, _) => {
            // AppG rule 8
            radicals::make_vcenter(ctx, &noad.nucleus)
        }
        (_, MathField::MathChar(ch) | MathField::MathTextChar(ch)) => make_character_nucleus(
            ctx,
            *ch,
            matches!(noad.nucleus, MathField::MathTextChar(_)),
            &noad.subscript,
            &mut delta,
        ),
        (_, MathField::Empty) => ctx.layout.empty(),
        (_, MathField::SubBox(list)) => source_list(ctx, *list),
        (_, MathField::SubMlist(list)) => {
            // TeX82's mlist2 branch always hpacks a sub-mlist nucleus. This
            // structural box is distinct from clean_box's later reuse of a
            // sole unshifted box around the completed field.
            let list = convert_mlist(ctx, *list, ctx.style, false);
            let boxed = ctx.layout.hpack(list);
            ctx.layout.hlist([MathNode::HList(boxed)])
        }
    };

    if !scripts_handled
        && (!matches!(noad.subscript, MathField::Empty)
            || !matches!(noad.superscript, MathField::Empty))
    {
        scripts::make_scripts(
            ctx,
            &mut hlist,
            &noad.subscript,
            &noad.superscript,
            ctx.style,
            delta,
        );
    }
    WorkNoad {
        class,
        hlist,
        penalty: match class {
            NoadClass::Bin => ctx.params.bin_op_penalty,
            NoadClass::Rel => ctx.params.rel_penalty,
            _ => INF_PENALTY,
        },
    }
}

fn second_pass<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    base_style: Style,
    work: Vec<WorkItem>,
    penalties: bool,
    max_height: Scaled,
    max_depth: Scaled,
) -> FrozenHList {
    // AppG rule 20
    let mut output = Vec::with_capacity(work.len().saturating_mul(2));
    let mut previous = None;
    let mut work = work.into_iter().peekable();
    while let Some(item) = work.next() {
        match item {
            WorkItem::Style(style) => ctx.set_style(style),
            WorkItem::Node(node) => output.push(node),
            WorkItem::Noad(noad) => {
                if let Some(left) = previous
                    && let spacing = spacing::inter_noad_spacing(left, noad.class, ctx.style)
                    && let Some(spec) = spacing::spacing_glue(spacing, ctx.params, ctx.mu)
                {
                    output.push(MathNode::Glue {
                        spec,
                        kind: math_glue_kind_for_spacing(spacing),
                        leader: None,
                    });
                }
                output.push(MathNode::Sequence(noad.hlist));
                if penalties
                    && noad.penalty < INF_PENALTY
                    && work.peek().is_some_and(|next| {
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
                    // AppG rule 21
                    output.push(MathNode::Penalty(noad.penalty));
                }
                previous = Some(noad.class);
            }
            WorkItem::Delimiter(delimiter) => {
                let class = delimiter.class;
                // AppG rule 19
                if let Some(left) = previous
                    && let spacing = spacing::inter_noad_spacing(left, delimiter.class, ctx.style)
                    && let Some(spec) = spacing::spacing_glue(spacing, ctx.params, ctx.mu)
                {
                    output.push(MathNode::Glue {
                        spec,
                        kind: math_glue_kind_for_spacing(spacing),
                        leader: None,
                    });
                }
                let target =
                    left_right_delimiter_target(ctx.params, base_style, max_height, max_depth);
                let delimiter =
                    delimiters::var_delimiter(ctx, delimiter.delimiter, base_style.size(), target);
                output.push(boxed_node(delimiter));
                previous = Some(class);
            }
        }
    }
    ctx.layout.hlist(output)
}

fn math_glue_kind_for_spacing(spacing: SpacingKind) -> MathGlueKind {
    match spacing {
        SpacingKind::None => MathGlueKind::MuSkip,
        SpacingKind::Thin => MathGlueKind::ThinMuSkip,
        SpacingKind::Med => MathGlueKind::MedMuSkip,
        SpacingKind::Thick => MathGlueKind::ThickMuSkip,
    }
}

pub(crate) fn clean_box(
    ctx: &mut Context<'_, impl MathTypesetState>,
    field: &MathField,
    style: Style,
) -> MathBox {
    // AppG rule 17
    match field {
        MathField::Empty => ctx.layout.hpack(ctx.layout.empty()),
        MathField::MathChar(ch) | MathField::MathTextChar(ch) => {
            if let Some(fetched) = fetch(ctx.state, *ch, style) {
                char_box(ctx, fetched)
            } else {
                ctx.layout.hpack(ctx.layout.empty())
            }
        }
        MathField::SubBox(list) => {
            let list = source_list(ctx, *list);
            clean_hlist(ctx, list)
        }
        MathField::SubMlist(list) => {
            let list = convert_mlist(ctx, *list, style, false);
            clean_hlist(ctx, list)
        }
    }
}

fn clean_hlist(ctx: &Context<'_, impl MathTypesetState>, list: FrozenHList) -> MathBox {
    match ctx.layout.single_node(list) {
        Some(MathNode::HList(boxed) | MathNode::VList(boxed)) if boxed.shift.raw() == 0 => {
            boxed.clone()
        }
        _ => ctx.layout.hpack(list),
    }
}

pub(crate) fn make_character_nucleus<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    ch: MathChar,
    text_char: bool,
    subscript: &MathField,
    delta: &mut Scaled,
) -> FrozenHList {
    // AppG rule 17
    let Some(fetched) = fetch(ctx.state, ch, ctx.style) else {
        return ctx.layout.empty();
    };
    *delta = fetched.metrics.italic_correction;
    if text_char && ctx.state.font_parameter(fetched.font, 2).raw() != 0 {
        *delta = Scaled::from_raw(0);
    }
    let character = MathNode::Char {
        font: fetched.font,
        ch: fetched.ch,
        metrics: fetched.metrics,
    };
    if matches!(subscript, MathField::Empty) && delta.raw() != 0 {
        let kern = MathNode::Kern {
            amount: *delta,
            kind: KernKind::Font,
        };
        *delta = Scaled::from_raw(0);
        ctx.layout.hlist([character, kern])
    } else {
        ctx.layout.hlist([character])
    }
}

pub(crate) fn char_box(
    ctx: &mut Context<'_, impl MathTypesetState>,
    fetched: FetchedChar,
) -> MathBox {
    // AppG rule 17
    let list = ctx.layout.hlist([MathNode::Char {
        font: fetched.font,
        ch: fetched.ch,
        metrics: fetched.metrics,
    }]);
    MathBox {
        width: add(fetched.metrics.width, fetched.metrics.italic_correction),
        height: fetched.metrics.height,
        depth: fetched.metrics.depth,
        shift: Scaled::from_raw(0),
        list,
        axis: BoxAxis::Horizontal,
        display: false,
        glue_set: tex_state::scaled::GlueSetRatio::from_raw(0),
        glue_sign: tex_state::node::Sign::Normal,
        glue_order: tex_state::glue::Order::Normal,
    }
}

pub(crate) fn fetch(
    state: &impl MathTypesetState,
    ch: MathChar,
    style: Style,
) -> Option<FetchedChar> {
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

pub(crate) fn source_list(
    ctx: &mut Context<'_, impl MathTypesetState>,
    list: NodeListId,
) -> FrozenHList {
    let source = ctx.state.nodes(list).to_vec();
    let nodes = source
        .iter()
        .map(|node| source_node(ctx, node))
        .collect::<Vec<_>>();
    ctx.layout.hlist(nodes)
}

pub(crate) fn source_node(ctx: &mut Context<'_, impl MathTypesetState>, node: &Node) -> MathNode {
    match node {
        Node::Char { font, ch } => {
            let code = u8::try_from(u32::from(*ch)).ok();
            if let Some(metrics) = code.and_then(|code| ctx.state.font_char_metrics(*font, code)) {
                MathNode::Char {
                    font: *font,
                    ch: *ch,
                    metrics,
                }
            } else {
                MathNode::Opaque(Box::new(node.clone()))
            }
        }
        Node::Kern { amount, kind } => MathNode::Kern {
            amount: *amount,
            kind: *kind,
        },
        Node::Glue { spec, kind, leader } => MathNode::Glue {
            spec: ctx.state.glue(*spec),
            kind: *kind,
            leader: *leader,
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
        Node::HList(box_node) | Node::VList(box_node) => {
            let list = source_list(ctx, box_node.children);
            let boxed = MathBox {
                width: box_node.width,
                height: box_node.height,
                depth: box_node.depth,
                shift: box_node.shift,
                list,
                axis: if matches!(node, Node::HList(_)) {
                    BoxAxis::Horizontal
                } else {
                    BoxAxis::Vertical
                },
                display: box_node.display,
                glue_set: box_node.glue_set,
                glue_sign: box_node.glue_sign,
                glue_order: box_node.glue_order,
            };
            if matches!(node, Node::HList(_)) {
                MathNode::HList(boxed)
            } else {
                MathNode::VList(boxed)
            }
        }
        _ => MathNode::Opaque(Box::new(node.clone())),
    }
}

fn noad_class(noad: &MathNoad) -> NoadClass {
    match noad.kind {
        NoadKind::Normal(class) => class,
        NoadKind::Operator(_) => NoadClass::Op,
        NoadKind::Radical { .. }
        | NoadKind::Accent { .. }
        | NoadKind::LeftDelimiter { .. }
        | NoadKind::RightDelimiter { .. }
        | NoadKind::MiddleDelimiter { .. }
        | NoadKind::Underline
        | NoadKind::Overline
        | NoadKind::VCenter => {
            // AppG rule 16
            NoadClass::Ord
        }
    }
}

fn convert_final_bin_to_ord(work: &mut [WorkItem]) {
    if let Some(WorkItem::Noad(noad)) = work
        .iter_mut()
        .rev()
        .find(|item| matches!(item, WorkItem::Noad(_)))
        && noad.class == NoadClass::Bin
    {
        // AppG rule 20
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

pub(crate) struct Context<'a, S> {
    pub(crate) state: &'a S,
    pub(crate) params: &'a MathParams,
    pub(crate) style: Style,
    pub(crate) mu: Scaled,
    pub(crate) layout: MathLayoutBuilder,
    pub(crate) converted: HashMap<(NodeListId, Style), FrozenHList>,
}

impl<S> Context<'_, S> {
    fn set_style(&mut self, style: Style) {
        self.style = style;
        self.mu = math_unit(self.params, style);
    }
}
use std::collections::{HashMap, HashSet};
