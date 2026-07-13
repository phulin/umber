use tex_fonts::CharMetrics;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::FontId;
use tex_state::node::{GlueKind, KernKind, LeaderPayload, Node, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

use super::{add, sub};

/// One converted math layout backed by a contiguous node arena.
#[derive(Clone, Debug, PartialEq)]
pub struct MathLayout {
    nodes: Vec<MathNode>,
    root: FrozenHList,
}

impl MathLayout {
    #[must_use]
    pub const fn root(&self) -> FrozenHList {
        self.root
    }

    #[must_use]
    pub fn nodes(&self, list: FrozenHList) -> &[MathNode] {
        let start = list.start as usize;
        let end = start + list.len as usize;
        assert!(end <= self.nodes.len(), "math layout span is not live");
        &self.nodes[start..end]
    }

    #[cfg(test)]
    pub(crate) fn logical_nodes(&self, list: FrozenHList) -> Vec<&MathNode> {
        let mut out = Vec::new();
        self.collect_logical_nodes(list, &mut out);
        out
    }

    #[cfg(test)]
    fn collect_logical_nodes<'a>(&'a self, list: FrozenHList, out: &mut Vec<&'a MathNode>) {
        for node in self.nodes(list) {
            match node {
                MathNode::Sequence(child) => self.collect_logical_nodes(*child, out),
                node => out.push(node),
            }
        }
    }
}

/// Read-only access to formula-local structural spans during sink lowering.
pub trait MathLayoutReader {
    fn math_nodes(&self, list: FrozenHList) -> &[MathNode];
}

impl MathLayoutReader for MathLayout {
    fn math_nodes(&self, list: FrozenHList) -> &[MathNode] {
        self.nodes(list)
    }
}

/// An immutable, measured horizontal-list span in a [`MathLayout`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenHList {
    start: u32,
    len: u32,
    node_count: u32,
    width: Scaled,
    height: Scaled,
    depth: Scaled,
}

impl Default for FrozenHList {
    fn default() -> Self {
        Self {
            start: 0,
            len: 0,
            node_count: 0,
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
        }
    }
}

impl FrozenHList {
    #[must_use]
    pub const fn width(self) -> Scaled {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> Scaled {
        self.height
    }

    #[must_use]
    pub const fn depth(self) -> Scaled {
        self.depth
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.node_count == 0
    }

    #[must_use]
    pub const fn node_count(self) -> usize {
        self.node_count as usize
    }
}

/// Owned hlist/vlist node used by the pure math kernel.
#[derive(Clone, Debug, PartialEq)]
pub enum MathNode {
    Char {
        font: FontId,
        ch: char,
        metrics: CharMetrics,
    },
    Kern {
        amount: Scaled,
        kind: KernKind,
    },
    Glue {
        spec: GlueSpec,
        kind: GlueKind,
        leader: Option<LeaderPayload>,
    },
    Penalty(i32),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    HList(MathBox),
    VList(MathBox),
    Opaque(Box<Node>),
    /// Transparent concatenation of an already-built earlier span.
    #[doc(hidden)]
    Sequence(FrozenHList),
}

/// Glue subtype retained without requiring a `GlueId`.
pub type MathGlueKind = GlueKind;

/// Owned box node whose children are stored in the surrounding layout arena.
#[derive(Clone, Debug, PartialEq)]
pub struct MathBox {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    /// TeX.web `shift_amount`: positive moves down in an hlist and right in a vlist.
    pub shift: Scaled,
    pub list: FrozenHList,
    pub axis: BoxAxis,
    pub display: bool,
    pub glue_set: GlueSetRatio,
    pub glue_sign: Sign,
    pub glue_order: Order,
}

/// Box orientation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BoxAxis {
    Horizontal,
    Vertical,
}

pub(crate) struct MathLayoutBuilder {
    nodes: Vec<MathNode>,
}

impl MathLayoutBuilder {
    pub(crate) fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    pub(crate) fn finish(self, root: FrozenHList) -> MathLayout {
        debug_assert!(
            root.end() <= self.nodes.len(),
            "math layout root must belong to this arena"
        );
        MathLayout {
            nodes: self.nodes,
            root,
        }
    }

    pub(crate) fn empty(&self) -> FrozenHList {
        FrozenHList::default()
    }

    pub(crate) fn hlist(&mut self, nodes: impl IntoIterator<Item = MathNode>) -> FrozenHList {
        let start = self.nodes.len();
        self.nodes.extend(nodes);
        let end = self.nodes.len();
        for node in &self.nodes[start..end] {
            let child = match node {
                MathNode::Sequence(child) => Some(*child),
                MathNode::HList(boxed) | MathNode::VList(boxed) => Some(boxed.list),
                _ => None,
            };
            debug_assert!(
                child.is_none_or(|list| list.end() <= start),
                "math arena references must point to an earlier span"
            );
        }
        let mut meas = Measurement::ZERO;
        self.measure_hnodes(start, end, &mut meas);
        self.span(start, end, meas)
    }

    pub(crate) fn hpack(&self, list: FrozenHList) -> MathBox {
        MathBox {
            width: list.width,
            height: list.height,
            depth: list.depth,
            shift: Scaled::from_raw(0),
            list,
            axis: BoxAxis::Horizontal,
            display: false,
            glue_set: GlueSetRatio::from_raw(0),
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
        }
    }

    pub(crate) fn vpack(&self, list: FrozenHList) -> MathBox {
        let mut meas = Measurement::ZERO;
        self.measure_vnodes(list, &mut meas);
        MathBox {
            width: meas.width,
            height: meas.height,
            depth: meas.depth,
            shift: Scaled::from_raw(0),
            list,
            axis: BoxAxis::Vertical,
            display: false,
            glue_set: GlueSetRatio::from_raw(0),
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
        }
    }

    pub(crate) fn nodes(&self, list: FrozenHList) -> &[MathNode] {
        let start = list.start as usize;
        let end = start + list.len as usize;
        &self.nodes[start..end]
    }

    pub(crate) fn first_node(&self, list: FrozenHList) -> Option<&MathNode> {
        for node in self.nodes(list) {
            match node {
                MathNode::Sequence(child) => {
                    if let Some(node) = self.first_node(*child) {
                        return Some(node);
                    }
                }
                node => return Some(node),
            }
        }
        None
    }

    pub(crate) fn single_node(&self, list: FrozenHList) -> Option<&MathNode> {
        if list.node_count == 1 {
            self.first_node(list)
        } else {
            None
        }
    }

    fn span(&self, start: usize, end: usize, meas: Measurement) -> FrozenHList {
        FrozenHList {
            start: u32::try_from(start).expect("math layout exceeds u32 nodes"),
            len: u32::try_from(end - start).expect("math list exceeds u32 nodes"),
            node_count: meas.node_count,
            width: meas.width,
            height: meas.height,
            depth: meas.depth,
        }
    }

    fn measure_hnodes(&self, start: usize, end: usize, meas: &mut Measurement) {
        for node in &self.nodes[start..end] {
            match node {
                MathNode::Sequence(list) => {
                    meas.node_count = meas.node_count.saturating_add(list.node_count);
                    meas.width = add(meas.width, list.width);
                    meas.height = meas.height.max(list.height);
                    meas.depth = meas.depth.max(list.depth);
                }
                MathNode::Char { metrics, .. } => {
                    meas.node_count = meas.node_count.saturating_add(1);
                    meas.width = add(meas.width, metrics.width);
                    meas.height = meas.height.max(metrics.height);
                    meas.depth = meas.depth.max(metrics.depth);
                }
                MathNode::Kern { amount, .. } => {
                    meas.node_count = meas.node_count.saturating_add(1);
                    meas.width = add(meas.width, *amount);
                }
                MathNode::Glue { spec, .. } => {
                    meas.node_count = meas.node_count.saturating_add(1);
                    meas.width = add(meas.width, spec.width);
                }
                MathNode::Penalty(_) => meas.node_count = meas.node_count.saturating_add(1),
                MathNode::Rule {
                    width,
                    height,
                    depth,
                } => {
                    meas.node_count = meas.node_count.saturating_add(1);
                    meas.width = add(meas.width, width.unwrap_or(Scaled::from_raw(0)));
                    meas.height = meas.height.max(height.unwrap_or(Scaled::from_raw(0)));
                    meas.depth = meas.depth.max(depth.unwrap_or(Scaled::from_raw(0)));
                }
                MathNode::HList(boxed) | MathNode::VList(boxed) => {
                    meas.node_count = meas.node_count.saturating_add(1);
                    meas.width = add(meas.width, boxed.width);
                    meas.height = meas.height.max(sub(boxed.height, boxed.shift));
                    meas.depth = meas.depth.max(add(boxed.depth, boxed.shift));
                }
                MathNode::Opaque(node) => {
                    meas.node_count = meas.node_count.saturating_add(1);
                    measure_opaque_hnode(node, meas);
                }
            }
        }
    }

    fn measure_vnodes(&self, list: FrozenHList, meas: &mut Measurement) {
        for node in self.nodes(list) {
            match node {
                MathNode::Sequence(child) => self.measure_vnodes(*child, meas),
                MathNode::HList(boxed) | MathNode::VList(boxed) => {
                    meas.height = add(add(meas.height, meas.depth), boxed.height);
                    meas.depth = boxed.depth;
                    meas.width = meas.width.max(add(boxed.width, boxed.shift));
                }
                MathNode::Kern { amount, .. } => {
                    meas.height = add(meas.height, add(meas.depth, *amount));
                    meas.depth = Scaled::from_raw(0);
                }
                MathNode::Glue { spec, .. } => {
                    meas.height = add(meas.height, add(meas.depth, spec.width));
                    meas.depth = Scaled::from_raw(0);
                }
                MathNode::Rule {
                    width,
                    height,
                    depth,
                } => {
                    meas.height = add(
                        add(meas.height, meas.depth),
                        height.unwrap_or(Scaled::from_raw(0)),
                    );
                    meas.depth = depth.unwrap_or(Scaled::from_raw(0));
                    meas.width = meas.width.max(width.unwrap_or(Scaled::from_raw(0)));
                }
                MathNode::Penalty(_) | MathNode::Char { .. } | MathNode::Opaque(_) => {}
            }
        }
    }
}

impl FrozenHList {
    fn end(self) -> usize {
        self.start as usize + self.len as usize
    }
}

pub(crate) fn boxed_node(boxed: MathBox) -> MathNode {
    match boxed.axis {
        BoxAxis::Horizontal => MathNode::HList(boxed),
        BoxAxis::Vertical => MathNode::VList(boxed),
    }
}

pub(crate) fn hlist_extents(list: FrozenHList) -> (Scaled, Scaled) {
    (list.height, list.depth)
}

pub(crate) fn node_is_char(node: &MathNode) -> bool {
    matches!(node, MathNode::Char { .. })
}

fn measure_opaque_hnode(node: &Node, meas: &mut Measurement) {
    match node {
        Node::HList(boxed) | Node::VList(boxed) => {
            meas.width = add(meas.width, boxed.width);
            meas.height = meas.height.max(sub(boxed.height, boxed.shift));
            meas.depth = meas.depth.max(add(boxed.depth, boxed.shift));
        }
        _ => {}
    }
}

#[derive(Clone, Copy, Debug)]
struct Measurement {
    node_count: u32,
    width: Scaled,
    height: Scaled,
    depth: Scaled,
}

impl Measurement {
    const ZERO: Self = Self {
        node_count: 0,
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
    };
}
