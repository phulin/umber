use tex_fonts::CharMetrics;
use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;

/// Owned, immutable hlist produced by `mlist_to_hlist`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrozenHList {
    pub nodes: Vec<MathNode>,
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
        kind: MathGlueKind,
    },
    Penalty(i32),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    HList(MathBox),
    VList(MathBox),
    Opaque(Node),
}

/// Glue subtype retained without requiring a `GlueId`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MathGlueKind {
    Normal,
    MuSkip,
    ThinMuSkip,
    MedMuSkip,
    ThickMuSkip,
    NonScript,
    Source,
}

/// Owned box node.
#[derive(Clone, Debug, PartialEq)]
pub struct MathBox {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub shift: Scaled,
    pub list: FrozenHList,
    pub axis: BoxAxis,
}

/// Box orientation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BoxAxis {
    Horizontal,
    Vertical,
}

pub(crate) fn hpack(list: FrozenHList) -> MathBox {
    // AppG rule 17
    let meas = measure_hlist(&list);
    MathBox {
        width: meas.width,
        height: meas.height,
        depth: meas.depth,
        shift: Scaled::from_raw(0),
        list,
        axis: BoxAxis::Horizontal,
    }
}

pub(crate) fn hlist_extents(list: &FrozenHList) -> (Scaled, Scaled) {
    let meas = measure_hlist(list);
    (meas.height, meas.depth)
}

pub(crate) fn vpack(list: FrozenHList) -> MathBox {
    // AppG rule 18d
    let meas = measure_vlist(&list);
    MathBox {
        width: meas.width,
        height: meas.height,
        depth: meas.depth,
        shift: Scaled::from_raw(0),
        list,
        axis: BoxAxis::Vertical,
    }
}

pub(crate) fn boxed_node(boxed: MathBox) -> MathNode {
    match boxed.axis {
        BoxAxis::Horizontal => MathNode::HList(boxed),
        BoxAxis::Vertical => MathNode::VList(boxed),
    }
}

pub(crate) fn node_is_char(node: &MathNode) -> bool {
    matches!(node, MathNode::Char { .. })
}

fn measure_hlist(list: &FrozenHList) -> Measurement {
    let mut meas = Measurement::ZERO;
    for node in &list.nodes {
        match node {
            MathNode::Char { metrics, .. } => {
                meas.width = add(meas.width, metrics.width);
                meas.height = meas.height.max(metrics.height);
                meas.depth = meas.depth.max(metrics.depth);
            }
            MathNode::Kern { amount, .. } => meas.width = add(meas.width, *amount),
            MathNode::Glue { spec, .. } => meas.width = add(meas.width, spec.width),
            MathNode::Penalty(_) => {}
            MathNode::Rule {
                width,
                height,
                depth,
            } => {
                meas.width = add(meas.width, width.unwrap_or(Scaled::from_raw(0)));
                meas.height = meas.height.max(height.unwrap_or(Scaled::from_raw(0)));
                meas.depth = meas.depth.max(depth.unwrap_or(Scaled::from_raw(0)));
            }
            MathNode::HList(boxed) | MathNode::VList(boxed) => {
                meas.width = add(meas.width, boxed.width);
                meas.height = meas.height.max(add(boxed.height, boxed.shift));
                meas.depth = meas.depth.max(sub(boxed.depth, boxed.shift));
            }
            MathNode::Opaque(node) => measure_opaque_hnode(node, &mut meas),
        }
    }
    meas
}

fn measure_vlist(list: &FrozenHList) -> Measurement {
    let mut meas = Measurement::ZERO;
    for node in &list.nodes {
        match node {
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
    meas
}

fn measure_opaque_hnode(node: &Node, meas: &mut Measurement) {
    match node {
        Node::HList(boxed) | Node::VList(boxed) => {
            meas.width = add(meas.width, boxed.width);
            meas.height = meas.height.max(add(boxed.height, boxed.shift));
            meas.depth = meas.depth.max(sub(boxed.depth, boxed.shift));
        }
        _ => {}
    }
}

#[derive(Clone, Copy, Debug)]
struct Measurement {
    width: Scaled,
    height: Scaled,
    depth: Scaled,
}

impl Measurement {
    const ZERO: Self = Self {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
    };
}

fn add(left: Scaled, right: Scaled) -> Scaled {
    Scaled::from_raw(left.raw().saturating_add(right.raw()))
}

fn sub(left: Scaled, right: Scaled) -> Scaled {
    Scaled::from_raw(left.raw().saturating_sub(right.raw()))
}
