use tex_arith::WideScaled;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::Node;
use tex_state::node_arena::{NodeCursor, NodeRef, PackedNode, PageListId};
use tex_state::scaled::Scaled;

use crate::TypesetState;
use crate::expansion::ExpansionCapacity;
use crate::metrics::{MetricEvent, WideMetricTotals, wide_add_scaled};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Widths {
    base: WideMetricTotals,
    pub(super) font_stretch: WideScaled,
    pub(super) font_shrink: WideScaled,
}

impl Widths {
    pub(super) fn zero() -> Self {
        Self {
            base: WideMetricTotals::ZERO,
            font_stretch: WideScaled::ZERO,
            font_shrink: WideScaled::ZERO,
        }
    }

    pub(super) fn add_assign(&mut self, other: Self) {
        self.base.add_assign(other.base);
        self.font_stretch = wide_add(self.font_stretch, other.font_stretch);
        self.font_shrink = wide_add(self.font_shrink, other.font_shrink);
    }

    pub(super) fn from_glue(spec: GlueSpec) -> Self {
        let mut widths = Self::zero();
        add_glue(&mut widths, spec);
        widths
    }

    pub(super) fn sub(self, other: Self) -> Self {
        let mut out = Self::zero();
        out.base = self.base.sub(other.base);
        out.font_stretch = wide_sub(self.font_stretch, other.font_stretch);
        out.font_shrink = wide_sub(self.font_shrink, other.font_shrink);
        out
    }

    pub(super) fn normal_stretch(self) -> WideScaled {
        self.stretch[Order::Normal as usize]
    }

    pub(super) fn add_normal_stretch(&mut self, amount: Scaled) {
        self.stretch[Order::Normal as usize] = wide_add(
            self.stretch[Order::Normal as usize],
            WideScaled::from_scaled(amount),
        );
    }

    pub(super) fn normal_shrink(self) -> WideScaled {
        self.shrink[Order::Normal as usize]
    }

    pub(super) fn infinite_stretch(self) -> [WideScaled; 3] {
        [self.stretch[1], self.stretch[2], self.stretch[3]]
    }

    pub(super) fn infinite_stretch_is_zero(self) -> bool {
        self.infinite_stretch().iter().all(|value| value.raw() == 0)
    }

    pub(super) fn has_infinite_adjustment(self, shortfall: i64) -> bool {
        if shortfall > 0 {
            !self.infinite_stretch_is_zero()
        } else if shortfall < 0 {
            self.shrink[1..].iter().any(|value| value.raw() != 0)
        } else {
            false
        }
    }
}

impl std::ops::Deref for Widths {
    type Target = WideMetricTotals;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl std::ops::DerefMut for Widths {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

fn wide_add(left: WideScaled, right: WideScaled) -> WideScaled {
    left.checked_add(right)
        .expect("scaled accumulator exceeds the addressable node-list domain")
}

fn wide_sub(left: WideScaled, right: WideScaled) -> WideScaled {
    left.checked_sub(right)
        .expect("scaled accumulator exceeds the addressable node-list domain")
}

fn add_scaled(total: WideScaled, value: Scaled) -> WideScaled {
    wide_add_scaled(total, value)
}

pub(super) fn line_widths_view<S: TypesetState>(
    state: &S,
    list: &PageListId,
    start: usize,
    end: usize,
    include_font_expansion: bool,
) -> Widths {
    line_widths_cursor(
        state,
        state.page_nodes(*list),
        start,
        end,
        include_font_expansion,
    )
}

pub(super) fn line_widths_cursor<S: TypesetState>(
    state: &S,
    nodes: NodeCursor<'_>,
    start: usize,
    end: usize,
    include_font_expansion: bool,
) -> Widths {
    let mut widths = Widths::zero();
    let limit = end.min(nodes.len());
    let mut index = start.min(limit);
    while index < limit {
        if let Some(run) = nodes.char_codes(index) {
            let font = run.font();
            let mut run_len = 0;
            for code in run.take(limit - index) {
                // Preserve the scalar saturating-add order exactly.
                let natural = if state.font_uses_tfm_metrics(font) {
                    state.font_widths(font)[usize::from(code)]
                } else {
                    state
                        .font_character_metrics(font, char::from(code))
                        .map_or(Scaled::from_raw(0), |metrics| metrics.width)
                };
                widths.natural = add_scaled(widths.natural, natural);
                if include_font_expansion {
                    add_char_expansion(state, &mut widths, font, code, natural);
                }
                run_len += 1;
            }
            index += run_len;
        } else {
            add_node_width_source(&mut widths, state, nodes, index, include_font_expansion);
            index += 1;
        }
    }
    widths
}

#[cfg(test)]
pub(super) fn line_widths_nodes<S: TypesetState>(state: &S, nodes: &[Node]) -> Widths {
    let mut widths = Widths::zero();
    for index in 0..nodes.len() {
        add_node_width(&mut widths, state, nodes, index, true);
    }
    widths
}

#[cfg(test)]
pub(super) fn add_node_width<S: TypesetState>(
    widths: &mut Widths,
    state: &S,
    nodes: &[Node],
    index: usize,
    include_font_expansion: bool,
) {
    add_node_width_source(
        widths,
        state,
        NodeCursor::owned(nodes),
        index,
        include_font_expansion,
    );
}

pub(super) fn add_node_width_source<S: TypesetState>(
    widths: &mut Widths,
    state: &S,
    nodes: NodeCursor<'_>,
    index: usize,
    include_font_expansion: bool,
) {
    let Some(node) = nodes.owned_node(index) else {
        return;
    };
    add_node_width_value(
        widths,
        state,
        node,
        index
            .checked_sub(1)
            .and_then(|index| nodes.owned_node(index)),
        nodes.owned_node(index + 1),
        include_font_expansion,
    );
}

pub(super) fn add_node_width_value<S: TypesetState>(
    widths: &mut Widths,
    state: &S,
    node: &Node,
    previous: Option<&Node>,
    next: Option<&Node>,
    include_font_expansion: bool,
) {
    match NodeRef::from(node).packed() {
        PackedNode::Glyph { font, ch } => {
            if let Some(metrics) = state.font_character_metrics(font, ch) {
                widths.natural = add_scaled(widths.natural, metrics.width);
                if include_font_expansion && let Ok(code) = u8::try_from(ch as u32) {
                    add_char_expansion(state, widths, font, code, metrics.width);
                }
            }
        }
        PackedNode::Kern { amount, kind } => {
            // OpenType pass-1 cluster advances are carried as Font kern
            // adjustments beside their source character nodes. Counting the
            // adjustment here makes the accumulated width equal the shaped
            // cluster advance rather than the sum of cmap glyph advances.
            widths.natural = add_scaled(widths.natural, amount);
            if include_font_expansion && kind == Some(tex_state::node::KernKind::Font) {
                add_font_kern_expansion(state, widths, previous, next, amount);
            }
        }
        PackedNode::Math(width) => widths.natural = add_scaled(widths.natural, width),
        PackedNode::Glue { spec, .. } => add_glue(widths, spec),
        PackedNode::Rule { width, .. } => {
            if let Some(width) = width {
                widths.natural = add_scaled(widths.natural, width);
            }
        }
        PackedNode::Box(box_node) => {
            widths.natural = add_scaled(widths.natural, box_node.width);
        }
        PackedNode::Unset(unset) => {
            widths.natural = add_scaled(widths.natural, unset.width);
        }
        PackedNode::Disc(replace) => {
            add_nested_list_widths(widths, state, &replace, include_font_expansion);
        }
        PackedNode::Image { width, .. } => {
            widths.natural = add_scaled(widths.natural, width);
        }
        PackedNode::Ignored => {}
    }
}

/// Adds discretionary replacement lists without using the native call stack.
/// TeX node lists may be tens of thousands of levels deep, while the explicit
/// cursor stack remains proportional to depth and has a small fixed frame.
fn add_nested_list_widths<S: TypesetState>(
    widths: &mut Widths,
    state: &S,
    owner: &PageListId,
    include_font_expansion: bool,
) {
    let mut stack = vec![(*owner, 0usize)];
    while let Some((owner, index)) = stack.last_mut() {
        let cursor = state.page_nodes(*owner);
        if *index >= cursor.len() {
            let _ = stack.pop();
            continue;
        }
        let current = *index;
        *index += 1;
        let node = cursor
            .get(current)
            .expect("nested width cursor position is in bounds");
        match node.packed() {
            PackedNode::Glyph { font, ch } => {
                if let Some(metrics) = state.font_character_metrics(font, ch) {
                    widths.natural = add_scaled(widths.natural, metrics.width);
                    if include_font_expansion && let Ok(code) = u8::try_from(ch as u32) {
                        add_char_expansion(state, widths, font, code, metrics.width);
                    }
                }
            }
            PackedNode::Kern { amount, kind } => {
                widths.natural = add_scaled(widths.natural, amount);
                if include_font_expansion && kind == Some(tex_state::node::KernKind::Font) {
                    add_font_kern_expansion(
                        state,
                        widths,
                        current
                            .checked_sub(1)
                            .and_then(|index| cursor.owned_node(index)),
                        cursor.owned_node(current + 1),
                        amount,
                    );
                }
            }
            PackedNode::Math(width) => widths.natural = add_scaled(widths.natural, width),
            PackedNode::Glue { spec, .. } => add_glue(widths, spec),
            PackedNode::Rule { width, .. } => {
                if let Some(width) = width {
                    widths.natural = add_scaled(widths.natural, width);
                }
            }
            PackedNode::Box(box_node) => {
                widths.natural = add_scaled(widths.natural, box_node.width);
            }
            PackedNode::Unset(unset) => {
                widths.natural = add_scaled(widths.natural, unset.width);
            }
            PackedNode::Disc(replace) => {
                stack.push((replace, 0));
            }
            PackedNode::Image { width, .. } => {
                widths.natural = add_scaled(widths.natural, width);
            }
            PackedNode::Ignored => {}
        }
    }
}

fn add_glue(widths: &mut Widths, spec: GlueSpec) {
    widths.base.observe(MetricEvent::Glue(spec));
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
    let capacity = ExpansionCapacity::for_metric(
        natural,
        spec,
        state.pdf_font_code(tex_state::font::PdfFontCode::Ef, font, code),
    );
    widths.font_stretch = add_scaled(widths.font_stretch, capacity.stretch);
    widths.font_shrink = add_scaled(widths.font_shrink, capacity.shrink);
}

fn add_font_kern_expansion<S: TypesetState>(
    state: &S,
    widths: &mut Widths,
    previous: Option<&Node>,
    next: Option<&Node>,
    natural: Scaled,
) {
    let Some((left_font, left)) = previous.map(NodeRef::from).and_then(glyph) else {
        return;
    };
    let Some((right_font, right)) = next.map(NodeRef::from).and_then(glyph) else {
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
    let efcode = state.pdf_font_code(tex_state::font::PdfFontCode::Ef, left_font, left);
    let endpoint = state.font_kern(left_font, left, right).unwrap_or(natural);
    let stretched = crate::expansion::scaled_at_ratio(endpoint, spec.stretch());
    let shrunk = crate::expansion::scaled_at_ratio(endpoint, -spec.shrink());
    let stretch = ((stretched.raw() - natural.raw()).max(0), efcode);
    let shrink = ((natural.raw() - shrunk.raw()).max(0), efcode);
    widths.font_stretch = add_scaled(
        widths.font_stretch,
        rounded_positive_ratio(stretch.0, stretch.1),
    );
    widths.font_shrink = add_scaled(
        widths.font_shrink,
        rounded_positive_ratio(shrink.0, shrink.1),
    );
}

fn glyph(node: NodeRef<'_>) -> Option<(tex_state::ids::FontId, u8)> {
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

pub(super) fn line_badness(
    widths: Widths,
    target: Scaled,
    emergency: Scaled,
    expansion_steps: Option<(i32, i32)>,
) -> i32 {
    let mut diff = i64::from(target.raw()) - widths.natural.raw();
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
            tex_badness_wide(
                diff,
                add_scaled(widths.stretch[Order::Normal as usize], emergency).raw(),
            )
        }
    } else {
        let shrink_order = highest_order(widths.shrink);
        if shrink_order != Order::Normal && widths.shrink[shrink_order as usize].raw() > 0 {
            0
        } else if diff.saturating_abs() > widths.shrink[Order::Normal as usize].raw() {
            crate::INF_BAD + 1
        } else {
            tex_badness_wide(diff.abs(), widths.shrink[Order::Normal as usize].raw())
        }
    }
}

fn expansion_adjusted_shortfall(shortfall: i64, capacity: i64, steps: i32) -> i64 {
    if capacity > shortfall && steps > 0 {
        (capacity / i64::from(steps)) / 2
    } else {
        shortfall
            .checked_sub(capacity)
            .expect("line shortfall fits the wide scaled domain")
    }
}

fn highest_order(values: [WideScaled; 4]) -> Order {
    for order in [Order::Filll, Order::Fill, Order::Fil, Order::Normal] {
        if values[order as usize].raw() != 0 {
            return order;
        }
    }
    Order::Normal
}

/// TeX.web section 108 badness with widened inputs. Prefix subtraction can
/// produce a value outside `Scaled`; such a line is simply maximally bad,
/// while ordinary inputs retain TeX's exact integer operation order.
fn tex_badness_wide(t: i64, s: i64) -> i32 {
    if t == 0 {
        0
    } else if s <= 0 {
        crate::INF_BAD
    } else {
        let r = if t <= 7_230_584 {
            (t * 297) / s
        } else if s >= 1_663_497 {
            t / (s / 297)
        } else {
            t
        };
        if r > 1290 {
            crate::INF_BAD
        } else {
            i32::try_from((r * r * r + 0o400000) / 0o1000000)
                .expect("bounded TeX badness fits i32")
                .min(crate::INF_BAD)
        }
    }
}
