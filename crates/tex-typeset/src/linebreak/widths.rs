use tex_state::glue::{GlueSpec, Order};
use tex_state::node::Node;
use tex_state::node_arena::{NodeList, NodeRef};
use tex_state::scaled::Scaled;

use crate::expansion::{ExpansionCapacity, FontExpansionSpec};
use crate::{TypesetState, badness};

use super::{add, sub_scaled};

#[derive(Clone, Copy, Debug)]
pub(super) struct Widths {
    pub(super) natural: Scaled,
    stretch: [Scaled; 4],
    shrink: [Scaled; 4],
    pub(super) font_stretch: Scaled,
    pub(super) font_shrink: Scaled,
    pub(super) expansion_step: Option<i32>,
    pub(super) expansion_stretch_limit: Option<i32>,
    pub(super) expansion_shrink_limit: Option<i32>,
}

impl Widths {
    pub(super) fn zero() -> Self {
        Self {
            natural: Scaled::from_raw(0),
            stretch: [Scaled::from_raw(0); 4],
            shrink: [Scaled::from_raw(0); 4],
            font_stretch: Scaled::from_raw(0),
            font_shrink: Scaled::from_raw(0),
            expansion_step: None,
            expansion_stretch_limit: None,
            expansion_shrink_limit: None,
        }
    }

    pub(super) fn add_assign(&mut self, other: Self) {
        self.natural = add(self.natural, other.natural);
        for order in 0..4 {
            self.stretch[order] = add(self.stretch[order], other.stretch[order]);
            self.shrink[order] = add(self.shrink[order], other.shrink[order]);
        }
        self.font_stretch = add(self.font_stretch, other.font_stretch);
        self.font_shrink = add(self.font_shrink, other.font_shrink);
        merge_expansion_metadata(self, other);
    }

    pub(super) fn from_glue(spec: GlueSpec) -> Self {
        let mut widths = Self::zero();
        add_glue(&mut widths, spec);
        widths
    }

    pub(super) fn sub(self, other: Self) -> Self {
        let mut out = Self::zero();
        out.natural = sub_scaled(self.natural, other.natural);
        for order in 0..4 {
            out.stretch[order] = sub_scaled(self.stretch[order], other.stretch[order]);
            out.shrink[order] = sub_scaled(self.shrink[order], other.shrink[order]);
        }
        out.font_stretch = sub_scaled(self.font_stretch, other.font_stretch);
        out.font_shrink = sub_scaled(self.font_shrink, other.font_shrink);
        out.expansion_step = self.expansion_step.or(other.expansion_step);
        out.expansion_stretch_limit = self
            .expansion_stretch_limit
            .or(other.expansion_stretch_limit);
        out.expansion_shrink_limit = self.expansion_shrink_limit.or(other.expansion_shrink_limit);
        out
    }

    pub(super) fn normal_stretch(self) -> Scaled {
        self.stretch[Order::Normal as usize]
    }

    pub(super) fn add_normal_stretch(&mut self, amount: Scaled) {
        self.stretch[Order::Normal as usize] = add(self.stretch[Order::Normal as usize], amount);
    }

    pub(super) fn normal_shrink(self) -> Scaled {
        self.shrink[Order::Normal as usize]
    }

    pub(super) fn infinite_stretch(self) -> [Scaled; 3] {
        [self.stretch[1], self.stretch[2], self.stretch[3]]
    }

    pub(super) fn infinite_stretch_is_zero(self) -> bool {
        self.infinite_stretch().iter().all(|value| value.raw() == 0)
    }

    pub(super) fn has_infinite_adjustment(self, shortfall: i32) -> bool {
        if shortfall > 0 {
            !self.infinite_stretch_is_zero()
        } else if shortfall < 0 {
            self.shrink[1..].iter().any(|value| value.raw() != 0)
        } else {
            false
        }
    }
}

pub(super) fn line_widths_view<S: TypesetState>(
    state: &S,
    nodes: NodeList<'_>,
    start: usize,
    end: usize,
) -> Widths {
    let mut widths = Widths::zero();
    let limit = end.min(nodes.len());
    let mut index = start.min(limit);
    while index < limit {
        if let Some(run) = nodes.char_codes(index) {
            let font = run.font();
            let table = state.font_widths(font);
            let mut run_len = 0;
            for code in run.take(limit - index) {
                // Preserve the scalar saturating-add order exactly.
                let natural = table[usize::from(code)];
                widths.natural = add(widths.natural, natural);
                add_char_expansion(state, &mut widths, font, code, natural);
                run_len += 1;
            }
            index += run_len;
        } else {
            widths.add_assign(node_width_ref_at(state, nodes, index));
            index += 1;
        }
    }
    widths
}

pub(super) fn line_widths_nodes<S: TypesetState>(state: &S, nodes: &[Node]) -> Widths {
    let mut widths = Widths::zero();
    for index in 0..nodes.len() {
        widths.add_assign(node_width_at(state, nodes, index));
    }
    widths
}

pub(super) fn node_width_at<S: TypesetState>(state: &S, nodes: &[Node], index: usize) -> Widths {
    let node = &nodes[index];
    let mut widths = Widths::zero();
    match node {
        Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => {
            if let Ok(code) = u8::try_from(*ch as u32)
                && let Some(metrics) = state.font_char_metrics(*font, code)
            {
                widths.natural = add(widths.natural, metrics.width);
                add_char_expansion(state, &mut widths, *font, code, metrics.width);
            }
        }
        Node::Kern { amount, kind } => {
            widths.natural = add(widths.natural, *amount);
            if *kind == tex_state::node::KernKind::Font {
                add_font_kern_expansion(state, &mut widths, nodes, index, *amount);
            }
        }
        Node::MathOn(width) | Node::MathOff(width) => widths.natural = add(widths.natural, *width),
        Node::Glue { spec, .. } => add_glue(&mut widths, state.glue(*spec)),
        Node::Rule { width, .. } => {
            if let Some(width) = width {
                widths.natural = add(widths.natural, *width);
            }
        }
        Node::HList(box_node) | Node::VList(box_node) => {
            widths.natural = add(widths.natural, box_node.width);
        }
        Node::Unset(unset) => {
            widths.natural = add(widths.natural, unset.width);
        }
        Node::Disc { replace, .. } => {
            widths.add_assign(line_widths_view(
                state,
                state.nodes(*replace),
                0,
                state.nodes(*replace).len(),
            ));
        }
        Node::Penalty(_)
        | Node::Mark { .. }
        | Node::Ins { .. }
        | Node::Whatsit(_)
        | Node::MathNoad(_)
        | Node::FractionNoad(_)
        | Node::MathStyle(_)
        | Node::MathChoice(_)
        | Node::MathList(_)
        | Node::Nonscript
        | Node::Direction(_)
        | Node::Adjust(_) => {}
    }
    widths
}

fn node_width_ref_at<S: TypesetState>(state: &S, nodes: NodeList<'_>, index: usize) -> Widths {
    let node = nodes.get(index).expect("index is within node list");
    let mut widths = Widths::zero();
    match node {
        NodeRef::Char { font, ch, .. } | NodeRef::Lig { font, ch, .. } => {
            if let Ok(code) = u8::try_from(ch as u32)
                && let Some(metrics) = state.font_char_metrics(font, code)
            {
                widths.natural = add(widths.natural, metrics.width);
                add_char_expansion(state, &mut widths, font, code, metrics.width);
            }
        }
        NodeRef::Kern { amount, kind } => {
            widths.natural = add(widths.natural, amount);
            if kind == tex_state::node::KernKind::Font {
                add_font_kern_expansion_ref(state, &mut widths, nodes, index, amount);
            }
        }
        NodeRef::MathOn(amount) | NodeRef::MathOff(amount) => {
            widths.natural = add(widths.natural, amount)
        }
        NodeRef::Glue { spec, .. } => add_glue(&mut widths, state.glue(spec)),
        NodeRef::Rule {
            width: Some(width), ..
        } => widths.natural = add(widths.natural, width),
        NodeRef::HList(box_node) | NodeRef::VList(box_node) => {
            widths.natural = add(widths.natural, box_node.width)
        }
        NodeRef::Unset(unset) => widths.natural = add(widths.natural, unset.width),
        NodeRef::Disc { replace, .. } => {
            let list = state.nodes(replace);
            widths.add_assign(line_widths_view(state, list, 0, list.len()));
        }
        _ => {}
    }
    widths
}

fn add_font_kern_expansion_ref<S: TypesetState>(
    state: &S,
    widths: &mut Widths,
    nodes: NodeList<'_>,
    index: usize,
    natural: Scaled,
) {
    let Some((left_font, left)) = index
        .checked_sub(1)
        .and_then(|i| nodes.get(i))
        .and_then(glyph_ref)
    else {
        return;
    };
    let Some((right_font, right)) = nodes.get(index + 1).and_then(glyph_ref) else {
        return;
    };
    add_font_kern_capacity(state, widths, left_font, left, right_font, right, natural);
}

fn add_glue(widths: &mut Widths, spec: GlueSpec) {
    widths.natural = add(widths.natural, spec.width);
    widths.stretch[spec.stretch_order as usize] =
        add(widths.stretch[spec.stretch_order as usize], spec.stretch);
    widths.shrink[spec.shrink_order as usize] =
        add(widths.shrink[spec.shrink_order as usize], spec.shrink);
}

fn add_char_expansion<S: TypesetState>(
    state: &S,
    widths: &mut Widths,
    font: tex_state::ids::FontId,
    code: u8,
    natural: Scaled,
) {
    let Some(spec) = state.font_expansion_spec(font) else {
        return;
    };
    observe_expansion_metadata(widths, spec);
    let capacity = ExpansionCapacity::for_metric(
        natural,
        spec,
        state.pdf_font_code(tex_state::font::PdfFontCode::Ef, font, code),
    );
    widths.font_stretch = add(widths.font_stretch, capacity.stretch);
    widths.font_shrink = add(widths.font_shrink, capacity.shrink);
}

fn add_font_kern_expansion<S: TypesetState>(
    state: &S,
    widths: &mut Widths,
    nodes: &[Node],
    index: usize,
    natural: Scaled,
) {
    let Some((left_font, left)) = index.checked_sub(1).and_then(|i| glyph(&nodes[i])) else {
        return;
    };
    let Some((right_font, right)) = nodes.get(index + 1).and_then(glyph) else {
        return;
    };
    add_font_kern_capacity(state, widths, left_font, left, right_font, right, natural);
}

fn add_font_kern_capacity<S: TypesetState>(
    state: &S,
    widths: &mut Widths,
    left_font: tex_state::ids::FontId,
    left: u8,
    right_font: tex_state::ids::FontId,
    right: u8,
    natural: Scaled,
) {
    if left_font != right_font {
        return;
    }
    let Some(spec) = state.font_expansion_spec(left_font) else {
        return;
    };
    observe_expansion_metadata(widths, spec);
    let efcode = state.pdf_font_code(tex_state::font::PdfFontCode::Ef, left_font, left);
    let endpoint = state.font_kern(left_font, left, right).unwrap_or(natural);
    let stretched = crate::expansion::scaled_at_ratio(endpoint, spec.stretch());
    let shrunk = crate::expansion::scaled_at_ratio(endpoint, -spec.shrink());
    let stretch = ((stretched.raw() - natural.raw()).max(0), efcode);
    let shrink = ((natural.raw() - shrunk.raw()).max(0), efcode);
    widths.font_stretch = add(
        widths.font_stretch,
        rounded_positive_ratio(stretch.0, stretch.1),
    );
    widths.font_shrink = add(
        widths.font_shrink,
        rounded_positive_ratio(shrink.0, shrink.1),
    );
}

fn glyph(node: &Node) -> Option<(tex_state::ids::FontId, u8)> {
    match node {
        Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => {
            u8::try_from(*ch as u32).ok().map(|code| (*font, code))
        }
        _ => None,
    }
}

fn glyph_ref(node: NodeRef<'_>) -> Option<(tex_state::ids::FontId, u8)> {
    match node {
        NodeRef::Char { font, ch, .. } | NodeRef::Lig { font, ch, .. } => {
            u8::try_from(ch as u32).ok().map(|code| (font, code))
        }
        _ => None,
    }
}

fn rounded_positive_ratio(value: i32, efcode: i32) -> Scaled {
    let value = i64::from(value.max(0));
    let efcode = i64::from(efcode.clamp(0, 1000));
    Scaled::from_raw(
        i32::try_from((value * efcode + 500) / 1000).expect("font kern capacity fits i32"),
    )
}

fn observe_expansion_metadata(widths: &mut Widths, spec: FontExpansionSpec) {
    widths.expansion_step.get_or_insert(spec.step());
    if spec.stretch() != 0 {
        widths.expansion_stretch_limit.get_or_insert(spec.stretch());
    }
    if spec.shrink() != 0 {
        widths.expansion_shrink_limit.get_or_insert(spec.shrink());
    }
}

fn merge_expansion_metadata(target: &mut Widths, other: Widths) {
    target.expansion_step = target.expansion_step.or(other.expansion_step);
    target.expansion_stretch_limit = target
        .expansion_stretch_limit
        .or(other.expansion_stretch_limit);
    target.expansion_shrink_limit = target
        .expansion_shrink_limit
        .or(other.expansion_shrink_limit);
}

pub(super) fn line_badness(
    widths: Widths,
    target: Scaled,
    emergency: Scaled,
    expansion_steps: Option<(i32, i32)>,
) -> i32 {
    let mut diff = target.raw() - widths.natural.raw();
    if let Some((stretch_steps, shrink_steps)) = expansion_steps {
        if diff > 0 && widths.font_stretch.raw() > 0 {
            diff = expansion_adjusted_shortfall(diff, widths.font_stretch.raw(), stretch_steps);
        } else if diff < 0 && widths.font_shrink.raw() > 0 {
            diff = -expansion_adjusted_shortfall(-diff, widths.font_shrink.raw(), shrink_steps);
        }
    }
    if diff >= 0 {
        let stretch_order = highest_order(widths.stretch);
        if stretch_order != Order::Normal && widths.stretch[stretch_order as usize].raw() > 0 {
            0
        } else {
            badness(
                Scaled::from_raw(diff),
                add(widths.stretch[Order::Normal as usize], emergency),
            )
        }
    } else {
        let shrink_order = highest_order(widths.shrink);
        if shrink_order != Order::Normal && widths.shrink[shrink_order as usize].raw() > 0 {
            0
        } else if diff.saturating_abs() > widths.shrink[Order::Normal as usize].raw() {
            crate::INF_BAD + 1
        } else {
            badness(
                Scaled::from_raw(diff.saturating_abs()),
                widths.shrink[Order::Normal as usize],
            )
        }
    }
}

fn expansion_adjusted_shortfall(shortfall: i32, capacity: i32, steps: i32) -> i32 {
    if capacity > shortfall && steps > 0 {
        (capacity / steps) / 2
    } else {
        shortfall.saturating_sub(capacity)
    }
}

fn highest_order(values: [Scaled; 4]) -> Order {
    for order in [Order::Filll, Order::Fill, Order::Fil, Order::Normal] {
        if values[order as usize].raw() != 0 {
            return order;
        }
    }
    Order::Normal
}
