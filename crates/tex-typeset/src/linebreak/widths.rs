use tex_state::glue::{GlueSpec, Order};
use tex_state::node::Node;
use tex_state::scaled::Scaled;

use crate::{TypesetState, badness};

use super::{add, sub_scaled};

#[derive(Clone, Copy, Debug)]
pub(super) struct Widths {
    pub(super) natural: Scaled,
    stretch: [Scaled; 4],
    shrink: [Scaled; 4],
}

impl Widths {
    pub(super) fn zero() -> Self {
        Self {
            natural: Scaled::from_raw(0),
            stretch: [Scaled::from_raw(0); 4],
            shrink: [Scaled::from_raw(0); 4],
        }
    }

    pub(super) fn add_assign(&mut self, other: Self) {
        self.natural = add(self.natural, other.natural);
        for order in 0..4 {
            self.stretch[order] = add(self.stretch[order], other.stretch[order]);
            self.shrink[order] = add(self.shrink[order], other.shrink[order]);
        }
    }

    fn sub(self, other: Self) -> Self {
        let mut out = Self::zero();
        out.natural = sub_scaled(self.natural, other.natural);
        for order in 0..4 {
            out.stretch[order] = sub_scaled(self.stretch[order], other.stretch[order]);
            out.shrink[order] = sub_scaled(self.shrink[order], other.shrink[order]);
        }
        out
    }
}

pub(super) struct PrefixWidths {
    widths: Vec<Widths>,
}

impl PrefixWidths {
    pub(super) fn new<S: TypesetState>(state: &S, nodes: &[Node]) -> Self {
        let mut widths = Vec::with_capacity(nodes.len() + 1);
        let mut current = Widths::zero();
        widths.push(current);
        for node in nodes {
            current.add_assign(node_width(state, node));
            widths.push(current);
        }
        Self { widths }
    }

    pub(super) fn between(&self, start: usize, end: usize) -> Widths {
        self.widths[end].sub(self.widths[start])
    }
}

pub(super) fn line_widths<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    start: usize,
    end: usize,
) -> Widths {
    let mut widths = Widths::zero();
    for node in &nodes[start..end.min(nodes.len())] {
        widths.add_assign(node_width(state, node));
    }
    widths
}

fn node_width<S: TypesetState>(state: &S, node: &Node) -> Widths {
    let mut widths = Widths::zero();
    match node {
        Node::Char { font, ch } | Node::Lig { font, ch, .. } => {
            if let Ok(code) = u8::try_from(*ch as u32)
                && let Some(metrics) = state.font_char_metrics(*font, code)
            {
                widths.natural = add(widths.natural, metrics.width);
            }
        }
        Node::Kern { amount, .. } => widths.natural = add(widths.natural, *amount),
        Node::Glue { spec, .. } => add_glue(&mut widths, state.glue(*spec)),
        Node::Rule { width, .. } => {
            if let Some(width) = width {
                widths.natural = add(widths.natural, *width);
            }
        }
        Node::HList(box_node) | Node::VList(box_node) => {
            widths.natural = add(widths.natural, box_node.width);
        }
        Node::Disc { replace, .. } => {
            widths.add_assign(line_widths(
                state,
                state.nodes(*replace),
                0,
                state.nodes(*replace).len(),
            ));
        }
        Node::Penalty(_)
        | Node::Unset
        | Node::Mark { .. }
        | Node::Ins { .. }
        | Node::Whatsit(_)
        | Node::MathOn
        | Node::MathOff
        | Node::MathNoad(_)
        | Node::FractionNoad(_)
        | Node::MathStyle(_)
        | Node::MathChoice(_)
        | Node::MathList(_)
        | Node::Nonscript
        | Node::Adjust(_) => {}
    }
    widths
}

fn add_glue(widths: &mut Widths, spec: GlueSpec) {
    widths.natural = add(widths.natural, spec.width);
    widths.stretch[spec.stretch_order as usize] =
        add(widths.stretch[spec.stretch_order as usize], spec.stretch);
    widths.shrink[spec.shrink_order as usize] =
        add(widths.shrink[spec.shrink_order as usize], spec.shrink);
}

pub(super) fn line_badness(widths: Widths, target: Scaled, emergency: Scaled) -> i32 {
    let diff = target.raw() - widths.natural.raw();
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

fn highest_order(values: [Scaled; 4]) -> Order {
    for order in [Order::Filll, Order::Fill, Order::Fil, Order::Normal] {
        if values[order as usize].raw() != 0 {
            return order;
        }
    }
    Order::Normal
}
