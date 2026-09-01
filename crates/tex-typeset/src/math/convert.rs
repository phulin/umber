use ahash::{AHashMap, AHashSet};
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use tex_arith::x_over_n;
use tex_fonts::CharMetrics;
use tex_state::font::NULL_FONT;
use tex_state::ids::FontId;
use tex_state::math::{LimitType, MathChar, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::{GlueKind, KernKind, Node};
use tex_state::node_arena::PageListId;
use tex_state::scaled::Scaled;

use super::{
    BoxAxis, FrozenHList, MathBox, MathConversionEvent, MathGlueKind, MathLayout, MathNode,
    MathPackObservation, MathParams, MathTypesetState, NativeBoxSource, NativeNodeEvidence,
    NativeNodeTransaction, SpacingKind, Style, StyleFamily, add, boxed_node, delimiters, fractions,
    left_right_delimiter_target, operators, radicals, scripts, spacing,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug)]
pub(crate) struct FetchedChar {
    pub(crate) font: FontId,
    pub(crate) ch: char,
    pub(crate) metrics: CharMetrics,
    pub(crate) glyph_id: Option<u16>,
    pub(crate) top_accent_attachment: Option<Scaled>,
}

const INF_PENALTY: i32 = 10_000;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum SourceListRole {
    HorizontalField,
    BoxPayload,
}

#[must_use]
pub fn mlist_to_hlist(
    state: &impl MathTypesetState,
    input: PageListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
) -> MathLayout {
    build_math_layout(state, input, style, penalties, params)
}

fn build_math_layout(
    state: &impl MathTypesetState,
    input: PageListId,
    style: Style,
    penalties: bool,
    params: &MathParams,
) -> MathLayout {
    let mut ctx = Context {
        state,
        params,
        style,
        mu: math_unit(params, style),
        layout: NativeNodeTransaction::new(),
        converted: AHashMap::new(),
        source_lists: AHashMap::new(),
        conversion_events: RefCell::new(Vec::new()),
        capture_replay: false,
        pack_replays: Vec::new(),
        event_replays: RefCell::new(Vec::new()),
        recovered: Cell::new(false),
        scratch: ConversionScratch::default(),
    };
    prepare_nested_mlists(&mut ctx, input, style);
    let root = convert_mlist_uncached(&mut ctx, input, style, penalties);
    let recovered = ctx.recovered.get();
    let root = if recovered { ctx.layout.empty() } else { root };
    ctx.layout
        .finish_with_conversion(root, ctx.conversion_events.into_inner(), recovered)
}

pub(super) fn convert_mlist<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    input: PageListId,
    style: Style,
    _penalties: bool,
) -> FrozenHList {
    let converted = ctx
        .converted
        .get(&(input, style))
        .expect("nested math list was not prepared by the iterative conversion planner")
        .clone();
    // TeX82 Appendix G recursively converts every sub-mlist occurrence. The
    // iterative planner may share its pure node layout, but §651's completed
    // hpacks are observable effects and must be replayed at every demand.
    if ctx.capture_replay {
        ctx.pack_replays.push(ReplayMarker {
            position: ctx.layout.pack_observation_count(),
            replay: converted.pack_observations.clone(),
        });
    } else {
        converted
            .pack_observations
            .for_each_leaf(|events| ctx.layout.replay_pack_observations(events));
    }
    // TeX82 Appendix G descends into this sub-mlist at this point.  The
    // iterative planner computes its pure layout bottom-up, so replay the
    // diagnostics captured during that computation at the recursive demand
    // site instead of leaking them in planner order.
    if ctx.capture_replay {
        let position = ctx.conversion_events.borrow().len();
        ctx.event_replays.borrow_mut().push(ReplayMarker {
            position,
            replay: converted.conversion_events.clone(),
        });
    } else {
        converted.conversion_events.for_each_leaf(|events| {
            ctx.conversion_events.borrow_mut().extend_from_slice(events);
        });
    }
    converted.list
}

#[derive(Clone)]
pub(crate) struct ConvertedMlist {
    list: FrozenHList,
    pack_observations: Replay<MathPackObservation>,
    conversion_events: Replay<MathConversionEvent>,
}

#[derive(Clone)]
pub(crate) struct Replay<T>(Option<Arc<ReplayNode<T>>>);

enum ReplayNode<T> {
    Leaf(Arc<[T]>),
    Sequence(Arc<[Replay<T>]>),
}

impl<T> Default for Replay<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> Replay<T> {
    fn leaf(events: &[T]) -> Self
    where
        T: Clone,
    {
        if events.is_empty() {
            Self::default()
        } else {
            Self(Some(Arc::new(ReplayNode::Leaf(Arc::from(events)))))
        }
    }

    fn sequence(mut parts: Vec<Self>) -> Self {
        parts.retain(|part| part.0.is_some());
        match parts.len() {
            0 => Self::default(),
            1 => parts.pop().expect("single replay part exists"),
            _ => Self(Some(Arc::new(ReplayNode::Sequence(parts.into())))),
        }
    }

    fn for_each_leaf(&self, mut visit: impl FnMut(&[T])) {
        let mut stack = vec![self];
        while let Some(replay) = stack.pop() {
            match replay.0.as_deref() {
                None => {}
                Some(ReplayNode::Leaf(events)) => visit(events),
                Some(ReplayNode::Sequence(parts)) => {
                    stack.extend(parts.iter().rev());
                }
            }
        }
    }
}

pub(crate) struct ReplayMarker<T> {
    position: usize,
    replay: Replay<T>,
}

fn captured_replay<T: Clone>(
    direct: Vec<T>,
    start: usize,
    markers: Vec<ReplayMarker<T>>,
) -> Replay<T> {
    if direct.is_empty() && markers.iter().all(|marker| marker.replay.0.is_none()) {
        return Replay::default();
    }
    let mut parts = Vec::with_capacity(markers.len().saturating_mul(2).saturating_add(1));
    let mut cursor = 0;
    for marker in markers {
        let position = marker
            .position
            .checked_sub(start)
            .expect("replay marker follows capture start");
        assert!(position >= cursor && position <= direct.len());
        parts.push(Replay::leaf(&direct[cursor..position]));
        parts.push(marker.replay);
        cursor = position;
    }
    parts.push(Replay::leaf(&direct[cursor..]));
    Replay::sequence(parts)
}

fn convert_mlist_uncached<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    input: PageListId,
    style: Style,
    penalties: bool,
) -> FrozenHList {
    let saved_style = ctx.style;
    ctx.set_style(style);
    let mut input_view = std::mem::take(&mut ctx.scratch.expansion);
    expand_math_choices_into(ctx.state, input, style, &mut input_view);
    let mut work = std::mem::take(&mut ctx.scratch.work);
    work.reserve(input_view.nodes.len().saturating_sub(work.capacity()));
    let mut max_height = Scaled::from_raw(0);
    let mut max_depth = Scaled::from_raw(0);
    first_pass(
        ctx,
        &mut input_view,
        style,
        &mut work,
        &mut max_height,
        &mut max_depth,
    );
    input_view.clear();
    ctx.scratch.expansion = input_view;
    convert_final_bin_to_ord(&mut work);
    let result = second_pass(ctx, style, &mut work, penalties, max_height, max_depth);
    debug_assert!(work.is_empty());
    ctx.scratch.work = work;
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
    left_class: NoadClass,
    right_class: NoadClass,
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
    view: &mut ExpandedMathView,
    base_style: Style,
    out: &mut Vec<WorkItem>,
    max_height: &mut Scaled,
    max_depth: &mut Scaled,
) {
    let mut style_marker = 0;
    let mut r_type = Some(NoadClass::Op);
    let mut index = 0;
    while index < view.len() {
        if matches!(
            view.node(ctx.state, index),
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
        ) && let Some(noad) = view.noad_mut(ctx.state, index)
        {
            noad.kind = NoadKind::Normal(NoadClass::Ord);
        }
        if matches!(
            view.node(ctx.state, index),
            Some(Node::MathNoad(noad)) if matches!(noad.kind, NoadKind::Normal(NoadClass::Ord))
        ) && operators::ord_pair_may_change(ctx, view, index)
        {
            operators::make_ord(ctx, view, index);
        }
        if let Some(children) = match view.node(ctx.state, index) {
            Some(Node::HList(boxed) | Node::VList(boxed)) => Some(boxed.children),
            _ => None,
        } {
            source_box_payload(ctx, children);
        }
        let state = ctx.state;
        match view
            .node(state, index)
            .expect("expanded math index remains in range")
        {
            Node::MathStyle(style) => {
                // AppG rule 3
                let full_style = view
                    .marker_styles
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
                let suppress_next = matches!(kind, GlueKind::NonScript)
                    && ctx.style.is_script_or_smaller()
                    && view
                        .node(state, index + 1)
                        .is_some_and(|next| matches!(next, Node::Glue { .. } | Node::Kern { .. }));
                if matches!(kind, tex_state::node::GlueKind::MuSkip) {
                    // TeX82 §732 converts both parts of an unconditional
                    // math-glue node: `math_glue` rewrites its specification
                    // and `subtype(q):=normal` records that the result is now
                    // ordinary glue. Named math spacing and leader subtypes do
                    // not enter this branch.
                    out.push(WorkItem::Node(MathNode::Glue {
                        spec: spacing::math_glue(*spec, ctx.mu),
                        kind: GlueKind::Normal,
                        leader: *leader,
                    }));
                } else {
                    out.push(WorkItem::Node(match view.source(index) {
                        Some(source) => native_source(source, NativeNodeEvidence::Glue(*spec)),
                        None => MathNode::Glue {
                            spec: *spec,
                            kind: *kind,
                            leader: *leader,
                        },
                    }));
                }
                // TeX82 §732 keeps the conditional-glue marker and removes
                // its glue/kern successor. Advance only after retaining the
                // marker's own source coordinate; otherwise lowering revives
                // the removed successor through that borrowed coordinate.
                if suppress_next {
                    index += 1;
                }
            }
            Node::Kern { amount, kind } => {
                // AppG rule 2
                if matches!(kind, KernKind::Mu) {
                    out.push(WorkItem::Node(MathNode::Kern {
                        amount: spacing::math_kern(*amount, ctx.mu),
                        kind: KernKind::Explicit,
                    }));
                } else {
                    out.push(WorkItem::Node(match view.source(index) {
                        Some(source) => native_source(source, NativeNodeEvidence::Kern(*amount)),
                        None => MathNode::Kern {
                            amount: *amount,
                            kind: *kind,
                        },
                    }));
                }
            }
            Node::MathNoad(noad)
                if matches!(
                    noad.kind,
                    NoadKind::LeftDelimiter { .. }
                        | NoadKind::RightDelimiter { .. }
                        | NoadKind::MiddleDelimiter { .. }
                ) =>
            {
                let (left_class, right_class, delimiter) = match noad.kind {
                    NoadKind::LeftDelimiter { delimiter } => {
                        (NoadClass::Open, NoadClass::Open, delimiter)
                    }
                    NoadKind::RightDelimiter { delimiter } => {
                        (NoadClass::Close, NoadClass::Close, delimiter)
                    }
                    NoadKind::MiddleDelimiter { delimiter } => {
                        (NoadClass::Close, NoadClass::Open, delimiter)
                    }
                    _ => unreachable!("guard restricts delimiter noads"),
                };
                if matches!(left_class, NoadClass::Close) {
                    // AppG rule 6
                    convert_final_bin_to_ord(out);
                }
                r_type = Some(right_class);
                out.push(WorkItem::Delimiter(WorkDelimiter {
                    left_class,
                    right_class,
                    delimiter,
                }));
                if matches!(
                    noad.kind,
                    NoadKind::RightDelimiter { .. } | NoadKind::MiddleDelimiter { .. }
                ) {
                    // e-TeX [36.727]: a right/middle noad restores the style
                    // in force at entry to this mlist, not the most recent
                    // explicit style node.
                    ctx.set_style(base_style);
                }
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
                // TeX82 §724's `check_dimensions` measures every completed
                // noad through `hpack(new_hlist(q), natural)`. This is an
                // observable pack even when the translated hlist is empty.
                let packed = ctx.layout.hpack(work.hlist);
                let (height, depth) = (packed.height, packed.depth);
                *max_height = (*max_height).max(height);
                *max_depth = (*max_depth).max(depth);
                r_type = Some(work.class);
                out.push(WorkItem::Noad(work));
            }
            Node::FractionNoad(fraction) => {
                // AppG rule 15
                let hlist = fractions::make_fraction(ctx, fraction);
                // Fractions rejoin the same §724 `check_dimensions` label as
                // ordinary noads and therefore complete the same natural pack.
                let packed = ctx.layout.hpack(hlist);
                let (height, depth) = (packed.height, packed.depth);
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
                out.push(WorkItem::Node(source_node(
                    ctx.state,
                    &ctx.source_lists,
                    view.source(index),
                    other,
                )));
            }
        }
        index += 1;
    }
}

/// Builds the immutable node view selected by Appendix G rule 4 without
/// recursively descending through nested `\mathchoice` lists.
#[derive(Default)]
pub(super) struct ExpandedMathView {
    nodes: Vec<ExpandedMathNode>,
    marker_styles: Vec<Style>,
    stack: Vec<ExpansionFrame>,
}

impl ExpandedMathView {
    fn clear(&mut self) {
        self.nodes.clear();
        self.marker_styles.clear();
        self.stack.clear();
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }

    fn source(&self, index: usize) -> Option<(PageListId, usize)> {
        match self.nodes.get(index)? {
            ExpandedMathNode::Source { list, index } => Some((*list, *index)),
            ExpandedMathNode::Owned(_) => None,
        }
    }

    pub(super) fn node<'a>(
        &'a self,
        state: &'a impl MathTypesetState,
        index: usize,
    ) -> Option<&'a Node> {
        match self.nodes.get(index)? {
            ExpandedMathNode::Source { list, index } => state.page_nodes(*list).owned_node(*index),
            ExpandedMathNode::Owned(node) => Some(node),
        }
    }

    pub(super) fn noad_mut<'a>(
        &'a mut self,
        state: &impl MathTypesetState,
        index: usize,
    ) -> Option<&'a mut MathNoad> {
        if let ExpandedMathNode::Source {
            list,
            index: source_index,
        } = self.nodes[index]
        {
            let Node::MathNoad(noad) = state
                .page_nodes(list)
                .owned_node(source_index)
                .expect("expanded math source remains live")
            else {
                return None;
            };
            self.nodes[index] = ExpandedMathNode::Owned(Node::MathNoad(MathNoad {
                kind: noad.kind.clone(),
                nucleus: noad.nucleus,
                subscript: noad.subscript,
                superscript: noad.superscript,
            }));
        }
        match &mut self.nodes[index] {
            ExpandedMathNode::Owned(Node::MathNoad(noad)) => Some(noad),
            ExpandedMathNode::Owned(_) => None,
            ExpandedMathNode::Source { .. } => unreachable!("source was detached for mutation"),
        }
    }

    pub(super) fn insert_owned(&mut self, index: usize, node: Node) {
        self.nodes.insert(index, ExpandedMathNode::Owned(node));
    }

    pub(super) fn remove(&mut self, index: usize) {
        self.nodes.remove(index);
    }
}

enum ExpandedMathNode {
    Source { list: PageListId, index: usize },
    Owned(Node),
}

#[derive(Clone)]
struct ExpansionFrame {
    list: PageListId,
    index: usize,
}

fn expand_math_choices_into(
    state: &impl MathTypesetState,
    root: PageListId,
    starting_style: Style,
    view: &mut ExpandedMathView,
) {
    view.clear();
    let mut style = starting_style;
    view.stack.push(ExpansionFrame {
        list: root,
        index: 0,
    });
    while let Some(frame) = view.stack.last_mut() {
        let Some(node) = state.page_nodes(frame.list).owned_node(frame.index) else {
            view.stack.pop();
            continue;
        };
        let source_list = frame.list;
        let source_index = frame.index;
        frame.index += 1;
        match node {
            Node::MathStyle(next) => {
                style = Style::from_math_style(*next);
                view.nodes.push(ExpandedMathNode::Source {
                    list: source_list,
                    index: source_index,
                });
                view.marker_styles.push(style);
            }
            Node::MathChoice(choice) => {
                // The style marker is semantically observable by the first
                // pass even though the choice itself disappears.
                view.nodes.push(ExpandedMathNode::Owned(Node::MathStyle(
                    match style.family() {
                        StyleFamily::Display => tex_state::math::MathStyle::Display,
                        StyleFamily::Text => tex_state::math::MathStyle::Text,
                        StyleFamily::Script => tex_state::math::MathStyle::Script,
                        StyleFamily::ScriptScript => tex_state::math::MathStyle::ScriptScript,
                    },
                )));
                view.marker_styles.push(style);
                let selected = match style.family() {
                    StyleFamily::Display => choice.display,
                    StyleFamily::Text => choice.text,
                    StyleFamily::Script => choice.script,
                    StyleFamily::ScriptScript => choice.script_script,
                };
                view.stack.push(ExpansionFrame {
                    list: selected,
                    index: 0,
                });
            }
            node => {
                let resets_style = matches!(
                    node,
                    Node::MathNoad(MathNoad {
                        kind: NoadKind::RightDelimiter { .. } | NoadKind::MiddleDelimiter { .. },
                        ..
                    })
                );
                view.nodes.push(ExpandedMathNode::Source {
                    list: source_list,
                    index: source_index,
                });
                if resets_style {
                    style = starting_style;
                }
            }
        }
    }
}

/// Converts structural sub-mlists bottom-up so Appendix G conversion never
/// follows a source-list edge on the Rust call stack. Math-choice branches are
/// scanned as inline views, matching rule 4, rather than converted separately.
fn prepare_nested_mlists<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    root: PageListId,
    root_style: Style,
) {
    let root = (root, root_style);
    let mut visiting = AHashSet::new();
    let mut completed = AHashSet::new();
    let mut stack = vec![(root, false)];
    let mut postorder = Vec::new();
    let mut requests = Vec::new();
    let mut request_seen = AHashSet::new();
    let mut request_view = std::mem::take(&mut ctx.scratch.expansion);
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
        nested_mlist_requests(
            ctx.state,
            list.0,
            list.1,
            &mut request_view,
            &mut requests,
            &mut request_seen,
        );
        for dependency in requests.iter().rev() {
            stack.push((*dependency, false));
        }
    }
    request_view.clear();
    ctx.scratch.expansion = request_view;

    for (list, style) in postorder.into_iter().filter(|key| key != &root) {
        let observation_start = ctx.layout.pack_observation_count();
        let event_start = ctx.conversion_events.borrow().len();
        ctx.capture_replay = true;
        debug_assert!(ctx.pack_replays.is_empty());
        debug_assert!(ctx.event_replays.borrow().is_empty());
        let converted = convert_mlist_uncached(ctx, list, style, false);
        ctx.capture_replay = false;
        let direct_pack_observations = ctx.layout.take_pack_observations_since(observation_start);
        let pack_observations = captured_replay(
            direct_pack_observations,
            observation_start,
            std::mem::take(&mut ctx.pack_replays),
        );
        let direct_conversion_events = ctx.conversion_events.borrow_mut().split_off(event_start);
        let conversion_events = captured_replay(
            direct_conversion_events,
            event_start,
            std::mem::take(&mut *ctx.event_replays.borrow_mut()),
        );
        ctx.converted.insert(
            (list, style),
            ConvertedMlist {
                list: converted,
                pack_observations,
                conversion_events,
            },
        );
    }
}

fn nested_mlist_requests(
    state: &impl MathTypesetState,
    root: PageListId,
    starting_style: Style,
    view: &mut ExpandedMathView,
    out: &mut Vec<(PageListId, Style)>,
    seen: &mut AHashSet<(PageListId, Style)>,
) {
    fn add_field(
        field: &MathField,
        style: Style,
        out: &mut Vec<(PageListId, Style)>,
        seen: &mut AHashSet<(PageListId, Style)>,
    ) {
        if let MathField::SubMlist(list) = field {
            let request = (*list, style);
            if seen.insert(request) {
                out.push(request);
            }
        }
    }

    expand_math_choices_into(state, root, starting_style, view);
    let mut style = starting_style;
    let mut markers = view.marker_styles.iter().copied();
    out.clear();
    seen.clear();
    for index in 0..view.len() {
        let node = view
            .node(state, index)
            .expect("expanded math request remains in range");
        match node {
            Node::MathStyle(_) => {
                style = markers
                    .next()
                    .expect("expanded style marker must retain its full style");
            }
            Node::MathNoad(noad)
                if matches!(
                    noad.kind,
                    NoadKind::LeftDelimiter { .. }
                        | NoadKind::RightDelimiter { .. }
                        | NoadKind::MiddleDelimiter { .. }
                ) =>
            {
                if matches!(
                    noad.kind,
                    NoadKind::RightDelimiter { .. } | NoadKind::MiddleDelimiter { .. }
                ) {
                    style = starting_style;
                }
            }
            Node::MathNoad(noad) => {
                let nucleus_style = if matches!(
                    noad.kind,
                    NoadKind::Radical { .. } | NoadKind::Accent { .. } | NoadKind::Overline
                ) {
                    style.cramped_style()
                } else {
                    style
                };
                add_field(&noad.nucleus, nucleus_style, out, seen);
                add_field(&noad.subscript, style.sub_style(), out, seen);
                add_field(&noad.superscript, style.sup_style(), out, seen);
            }
            Node::FractionNoad(fraction) => {
                add_field(
                    &MathField::SubMlist(fraction.numerator),
                    style.num_style(),
                    out,
                    seen,
                );
                add_field(
                    &MathField::SubMlist(fraction.denominator),
                    style.denom_style(),
                    out,
                    seen,
                );
            }
            _ => {}
        }
    }
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
        (_, MathField::SubBox(list)) => {
            // The source box crossed TeX82 §1086's package seam when it was
            // built. Appendix G reuses that completed box here; it does not
            // publish the historical hpack a second time.
            source_list(ctx, *list)
        }
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
    work: &mut Vec<WorkItem>,
    penalties: bool,
    max_height: Scaled,
    max_depth: Scaled,
) -> FrozenHList {
    // AppG rule 20
    let required = work
        .len()
        .checked_mul(2)
        .expect("math conversion capacity fits usize");
    let mut output = std::mem::take(&mut ctx.scratch.output);
    output.reserve(required.saturating_sub(output.capacity()));
    let mut previous = None;
    let mut items = work.drain(..).peekable();
    while let Some(item) = items.next() {
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
                    && items.peek().is_some_and(|next| {
                        !work_item_is_penalty(next)
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
                let right_class = delimiter.right_class;
                let resets_style = matches!(delimiter.left_class, NoadClass::Close);
                // AppG rule 19
                if let Some(left) = previous
                    && let spacing =
                        spacing::inter_noad_spacing(left, delimiter.left_class, ctx.style)
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
                previous = Some(right_class);
                if resets_style {
                    ctx.set_style(base_style);
                }
            }
        }
    }
    let result = ctx.layout.hlist(output.drain(..));
    ctx.scratch.output = output;
    result
}

fn work_item_is_penalty(item: &WorkItem) -> bool {
    matches!(
        item,
        WorkItem::Node(
            MathNode::Penalty(_)
                | MathNode::NativeSource {
                    evidence: NativeNodeEvidence::Penalty(_),
                    ..
                }
        )
    )
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
        // TeX82 §720 represents an empty field with `new_null_box`, then
        // recognizes that sole unshifted box as already clean. No hpack is
        // performed at this boundary.
        MathField::Empty => ctx.layout.null_hbox(),
        MathField::MathChar(ch) | MathField::MathTextChar(ch) => {
            if let Some(fetched) = fetch(ctx, *ch, style) {
                // TeX82 §720 does not return `char_box` directly here. It
                // converts a temporary one-noad mlist, then sends the
                // resulting character list through `hpack(q, natural)` at
                // `clean_box`'s common `found` branch. `char_box` has the
                // same finalized dimensions after the trivial italic-kern
                // simplification, but its construction alone does not cross
                // §651's observable hpack return seam.
                let boxed = char_box(ctx, fetched, ch.origin);
                // The shortcut stands in for two distinct TeX82 package
                // calls: the temporary one-noad mlist's §724 dimensions
                // check, followed by §720's common clean-box hpack.
                ctx.layout.observe_completed_pack(&boxed);
                ctx.layout.observe_completed_pack(&boxed);
                boxed
            } else {
                // `fetch` empties the temporary noad's field, but its §724
                // dimensions check and §720's common `hpack(q,natural)` both
                // still complete with the same null dimensions.
                let boxed = ctx.layout.null_hbox();
                ctx.layout.observe_completed_pack(&boxed);
                ctx.layout.observe_completed_pack(&boxed);
                boxed
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

fn clean_hlist(ctx: &mut Context<'_, impl MathTypesetState>, list: FrozenHList) -> MathBox {
    let mut boxed = match ctx.layout.single_node(list) {
        Some(MathNode::HList(boxed) | MathNode::VList(boxed)) if boxed.shift.raw() == 0 => *boxed,
        _ => ctx.layout.hpack(list),
    };
    // TeX82 §720's "Simplify a trivial box" physically unlinks the kern in
    // the exact character-plus-kern case after hpack has fixed the box
    // dimensions. This is semantic list ownership, not showbox normalization:
    // later consumers see the one-character payload while the packed width
    // still includes the removed italic correction.
    if let Some(character) = ctx.layout.trivial_character_before_kern(boxed.list) {
        boxed.list = ctx.layout.hlist([character]);
    }
    boxed
}

pub(crate) fn make_character_nucleus<S: MathTypesetState>(
    ctx: &mut Context<'_, S>,
    ch: MathChar,
    text_char: bool,
    subscript: &MathField,
    delta: &mut Scaled,
) -> FrozenHList {
    // AppG rule 17
    let Some(fetched) = fetch(ctx, ch, ctx.style) else {
        return ctx.layout.empty();
    };
    *delta = fetched.metrics.italic_correction;
    if text_char && ctx.state.font_parameter(fetched.font, 2).raw() != 0 {
        *delta = Scaled::from_raw(0);
    }
    let character = MathNode::Char {
        font: fetched.font,
        ch: fetched.ch,
        glyph_id: fetched.glyph_id,
        metrics: fetched.metrics,
        origin: ch.origin,
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
    origin: tex_state::token::OriginId,
) -> MathBox {
    // AppG rule 17
    let list = ctx.layout.hlist([MathNode::Char {
        font: fetched.font,
        ch: fetched.ch,
        glyph_id: fetched.glyph_id,
        metrics: fetched.metrics,
        origin,
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
        source: None,
    }
}

pub(crate) fn fetch(
    ctx: &Context<'_, impl MathTypesetState>,
    ch: MathChar,
    style: Style,
) -> Option<FetchedChar> {
    // AppG rule 17
    let font = ctx.state.math_family_font(style.size(), ch.family);
    if font == NULL_FONT {
        ctx.conversion_events
            .borrow_mut()
            .push(MathConversionEvent::UndefinedFamily {
                size: style.size(),
                family: ch.family,
                character: ch.character,
            });
        return None;
    }
    let fetched = match ctx.state.math_metrics_source(font) {
        tex_fonts::MathMetricsSource::OpenType(math) => math
            .glyph(ch.character, style.script_level())
            .map(|glyph| FetchedChar {
                font,
                ch: ch.character,
                metrics: glyph.metrics,
                glyph_id: Some(glyph.glyph_id),
                top_accent_attachment: glyph.top_accent_attachment,
            }),
        tex_fonts::MathMetricsSource::ClassicTfmExact => {
            u8::try_from(u32::from(ch.character)).ok().and_then(|code| {
                ctx.state
                    .classic_math_char_metrics(font, code)
                    .map(|metrics| FetchedChar {
                        font,
                        ch: ch.character,
                        metrics,
                        glyph_id: None,
                        top_accent_attachment: None,
                    })
            })
        }
    };
    if fetched.is_none() {
        ctx.conversion_events
            .borrow_mut()
            .push(MathConversionEvent::MissingCharacter {
                font,
                character: ch.character,
            });
    }
    fetched
}

pub(crate) fn source_list(
    ctx: &mut Context<'_, impl MathTypesetState>,
    list: PageListId,
) -> FrozenHList {
    convert_source_list(ctx, list, SourceListRole::HorizontalField)
}

pub(crate) fn source_box_payload(
    ctx: &mut Context<'_, impl MathTypesetState>,
    list: PageListId,
) -> FrozenHList {
    convert_source_list(ctx, list, SourceListRole::BoxPayload)
}

fn convert_source_list(
    ctx: &mut Context<'_, impl MathTypesetState>,
    list: PageListId,
    role: SourceListRole,
) -> FrozenHList {
    if let Some(converted) = ctx.source_lists.get(&(list, role)) {
        return *converted;
    }

    let mut stack = vec![(list, role, false)];
    let mut visiting = AHashSet::new();
    while let Some((current, current_role, expanded)) = stack.pop() {
        let key = (current, current_role);
        if ctx.source_lists.contains_key(&key) {
            continue;
        }
        if expanded {
            visiting.remove(&key);
            let start = ctx.layout.begin_direct_list();
            let state = ctx.state;
            let source_lists = &ctx.source_lists;
            let layout = &mut ctx.layout;
            let nodes = state.page_nodes(current);
            nodes.for_each_range(0..nodes.len(), |index, node| {
                layout.push_direct_node(source_node(
                    state,
                    source_lists,
                    Some((current, index)),
                    node,
                ));
            });
            let converted = match current_role {
                SourceListRole::HorizontalField => ctx.layout.finish_hlist(start),
                SourceListRole::BoxPayload => ctx.layout.finish_box_payload(start),
            };
            ctx.source_lists.insert(key, converted);
            continue;
        }
        assert!(
            visiting.insert(key),
            "source box lists must not contain structural cycles"
        );
        stack.push((current, current_role, true));
        let children_start = stack.len();
        let nodes = ctx.state.page_nodes(current);
        nodes.for_each(|node| {
            if let Node::HList(boxed) | Node::VList(boxed) = node {
                stack.push((boxed.children, SourceListRole::BoxPayload, false));
            }
        });
        // The explicit postorder stack pops the first source child first.
        stack[children_start..].reverse();
    }
    *ctx.source_lists
        .get(&(list, role))
        .expect("source-list postorder conversion must produce its root")
}

pub(crate) fn source_node(
    state: &impl MathTypesetState,
    source_lists: &AHashMap<(PageListId, SourceListRole), FrozenHList>,
    source: Option<(PageListId, usize)>,
    node: &Node,
) -> MathNode {
    match node {
        Node::Char { font, ch, origin } => {
            let code = u8::try_from(u32::from(*ch)).ok();
            let metrics = code.and_then(|code| state.classic_math_char_metrics(*font, code));
            match (source, metrics) {
                (Some(source), Some(metrics)) => {
                    native_source(source, NativeNodeEvidence::Character(metrics))
                }
                (Some(source), None) => native_source(source, NativeNodeEvidence::Inert),
                (None, Some(metrics)) => MathNode::Char {
                    font: *font,
                    ch: *ch,
                    glyph_id: None,
                    metrics,
                    origin: *origin,
                },
                (None, None) => {
                    panic!("coordinate-free math character must have classic metrics")
                }
            }
        }
        Node::Kern { amount, kind } => match source {
            Some(source) => native_source(source, NativeNodeEvidence::Kern(*amount)),
            None => MathNode::Kern {
                amount: *amount,
                kind: *kind,
            },
        },
        Node::Penalty(value) => match source {
            Some(source) => native_source(source, NativeNodeEvidence::Penalty(*value)),
            None => MathNode::Penalty(*value),
        },
        Node::Rule {
            width,
            height,
            depth,
        } => match source {
            Some(source) => native_source(
                source,
                NativeNodeEvidence::Rule {
                    width: *width,
                    height: *height,
                    depth: *depth,
                },
            ),
            None => MathNode::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
        },
        Node::Glue { spec, kind, leader } => match source {
            Some(source) => native_source(source, NativeNodeEvidence::Glue(*spec)),
            None => MathNode::Glue {
                spec: *spec,
                kind: *kind,
                leader: *leader,
            },
        },
        node @ (Node::HList(_) | Node::VList(_)) => {
            let horizontal = matches!(node, Node::HList(_));
            let box_node = match node {
                Node::HList(boxed) | Node::VList(boxed) => boxed,
                _ => unreachable!(),
            };
            let list = *source_lists
                .get(&(box_node.children, SourceListRole::BoxPayload))
                .expect("source box payload was prepared in postorder");
            let boxed = MathBox {
                width: box_node.width,
                height: box_node.height,
                depth: box_node.depth,
                shift: box_node.shift,
                list,
                axis: if horizontal {
                    BoxAxis::Horizontal
                } else {
                    BoxAxis::Vertical
                },
                display: box_node.box_lr == tex_state::node::BoxLr::DList,
                glue_set: box_node.glue_set,
                glue_sign: box_node.glue_sign,
                glue_order: box_node.glue_order,
                source: source.map(|(source_list, index)| NativeBoxSource {
                    list: source_list,
                    index: u32::try_from(index).expect("math source index fits u32"),
                    payload: list,
                }),
            };
            if horizontal {
                MathNode::HList(boxed)
            } else {
                MathNode::VList(boxed)
            }
        }
        _ => native_source(
            source.expect("generated math work contains only canonical draft node kinds"),
            NativeNodeEvidence::Inert,
        ),
    }
}

fn native_source((list, index): (PageListId, usize), evidence: NativeNodeEvidence) -> MathNode {
    MathNode::NativeSource {
        list,
        index: u32::try_from(index).expect("math source index fits u32"),
        evidence,
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
    pub(crate) layout: NativeNodeTransaction,
    pub(crate) converted: AHashMap<(PageListId, Style), ConvertedMlist>,
    pub(crate) source_lists: AHashMap<(PageListId, SourceListRole), FrozenHList>,
    pub(crate) conversion_events: RefCell<Vec<MathConversionEvent>>,
    pub(crate) capture_replay: bool,
    pub(crate) pack_replays: Vec<ReplayMarker<MathPackObservation>>,
    pub(crate) event_replays: RefCell<Vec<ReplayMarker<MathConversionEvent>>>,
    pub(crate) recovered: Cell<bool>,
    pub(crate) scratch: ConversionScratch,
}

#[derive(Default)]
pub(crate) struct ConversionScratch {
    expansion: ExpandedMathView,
    work: Vec<WorkItem>,
    output: Vec<MathNode>,
}

impl<S> Context<'_, S> {
    fn set_style(&mut self, style: Style) {
        self.style = style;
        self.mu = math_unit(self.params, style);
    }
}
