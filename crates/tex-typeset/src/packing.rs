use tex_state::Universe;
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::NodeListId;
use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
use tex_state::scaled::Scaled;

use crate::{INF_BAD, TypesetState, badness};

/// A requested box size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackSpec {
    /// Use the natural size of the list.
    Natural,
    /// Set the box to exactly this size.
    Exactly(Scaled),
    /// Add this amount to the list's natural size.
    Spread(Scaled),
}

/// Parameters used by horizontal packing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HpackParams {
    pub hbadness: i32,
    pub hfuzz: Scaled,
    pub overfull_rule: Scaled,
}

impl HpackParams {
    #[must_use]
    pub fn read(universe: &Universe) -> Self {
        Self {
            hbadness: universe.int_param(IntParam::HBADNESS),
            hfuzz: universe.dimen_param(DimenParam::HFUZZ),
            overfull_rule: universe.dimen_param(DimenParam::OVERFULL_RULE),
        }
    }
}

/// Parameters used by vertical packing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VpackParams {
    pub vbadness: i32,
    pub vfuzz: Scaled,
    pub box_max_depth: Scaled,
}

impl VpackParams {
    #[must_use]
    pub fn read(universe: &Universe) -> Self {
        Self {
            vbadness: universe.int_param(IntParam::VBADNESS),
            vfuzz: universe.dimen_param(DimenParam::VFUZZ),
            box_max_depth: universe.dimen_param(DimenParam::BOX_MAX_DEPTH),
        }
    }
}

/// Glue-setting diagnostics produced by packing.
#[derive(Clone, Debug, PartialEq)]
pub enum PackDiagnostic {
    Underfull { badness: i32, excess: Scaled },
    Loose { badness: i32, excess: Scaled },
    Tight { badness: i32, excess: Scaled },
    Overfull { excess: Scaled },
}

/// A packed hbox/vbox result.
#[derive(Clone, Debug, PartialEq)]
pub struct PackedBox {
    pub node: BoxNode,
    pub diagnostics: Vec<PackDiagnostic>,
}

#[must_use]
pub fn hpack(
    state: &impl TypesetState,
    list: NodeListId,
    spec: PackSpec,
    params: HpackParams,
) -> PackedBox {
    let meas = measure_hlist(state, state.nodes(list));
    let width = target_size(meas.width, spec);
    let glue = set_glue(width, meas.width, &meas);
    let diagnostics = hpack_diagnostics(width, meas.width, glue, params);
    PackedBox {
        node: BoxNode::new(BoxNodeFields {
            width,
            height: meas.height,
            depth: meas.depth,
            shift: Scaled::from_raw(0),
            glue_set: glue.ratio,
            glue_sign: glue.sign,
            glue_order: glue.order,
            children: list,
        }),
        diagnostics,
    }
}

#[must_use]
pub fn vpack(
    state: &impl TypesetState,
    list: NodeListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    let mut meas = measure_vlist(state, state.nodes(list));
    clamp_depth(&mut meas, params.box_max_depth);
    let height = target_size(meas.height, spec);
    let glue = set_glue(height, meas.height, &meas);
    let diagnostics = vpack_diagnostics(height, meas.height, glue, params);
    PackedBox {
        node: BoxNode::new(BoxNodeFields {
            width: meas.width,
            height,
            depth: meas.depth,
            shift: Scaled::from_raw(0),
            glue_set: glue.ratio,
            glue_sign: glue.sign,
            glue_order: glue.order,
            children: list,
        }),
        diagnostics,
    }
}

#[must_use]
pub fn vtop(
    state: &impl TypesetState,
    list: NodeListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    let mut packed = vpack(state, list, spec, params);
    let (height, depth) = vtop_split(state, list, packed.node.height, packed.node.depth);
    packed.node.height = height;
    packed.node.depth = depth;
    packed
}

#[derive(Clone, Copy, Debug)]
struct Measurement {
    width: Scaled,
    height: Scaled,
    depth: Scaled,
    stretch: [Scaled; 4],
    shrink: [Scaled; 4],
}

impl Measurement {
    const ZERO: Self = Self {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        stretch: [Scaled::from_raw(0); 4],
        shrink: [Scaled::from_raw(0); 4],
    };
}

#[derive(Clone, Copy, Debug)]
struct GlueSetting {
    ratio: f64,
    sign: Sign,
    order: Order,
    badness: i32,
    excess: Scaled,
}

fn target_size(natural: Scaled, spec: PackSpec) -> Scaled {
    match spec {
        PackSpec::Natural => natural,
        PackSpec::Exactly(size) => size,
        PackSpec::Spread(extra) => Scaled::from_raw(natural.raw().saturating_add(extra.raw())),
    }
}

fn set_glue(target: Scaled, natural: Scaled, meas: &Measurement) -> GlueSetting {
    let diff = target.raw() - natural.raw();
    if diff == 0 {
        return GlueSetting {
            ratio: 0.0,
            sign: Sign::Normal,
            order: Order::Normal,
            badness: 0,
            excess: Scaled::from_raw(0),
        };
    }
    let (sign, totals) = if diff > 0 {
        (Sign::Stretching, meas.stretch)
    } else {
        (Sign::Shrinking, meas.shrink)
    };
    let order = highest_order(totals);
    let total = totals[order as usize].raw();
    let excess = Scaled::from_raw(diff.abs());
    let ratio = if total == 0 {
        0.0
    } else if sign == Sign::Shrinking && order == Order::Normal && excess.raw() > total {
        1.0
    } else {
        f64::from(excess.raw()) / f64::from(total)
    };
    GlueSetting {
        ratio,
        sign,
        order,
        badness: badness(excess, Scaled::from_raw(total)),
        excess,
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

fn hpack_diagnostics(
    target: Scaled,
    natural: Scaled,
    glue: GlueSetting,
    params: HpackParams,
) -> Vec<PackDiagnostic> {
    common_diagnostics(target, natural, glue, params.hbadness, params.hfuzz)
}

fn vpack_diagnostics(
    target: Scaled,
    natural: Scaled,
    glue: GlueSetting,
    params: VpackParams,
) -> Vec<PackDiagnostic> {
    common_diagnostics(target, natural, glue, params.vbadness, params.vfuzz)
}

fn common_diagnostics(
    target: Scaled,
    natural: Scaled,
    glue: GlueSetting,
    badness_threshold: i32,
    fuzz: Scaled,
) -> Vec<PackDiagnostic> {
    let shortfall = natural.raw() - target.raw();
    if shortfall > fuzz.raw() && glue.sign == Sign::Shrinking && glue.badness >= INF_BAD {
        return vec![PackDiagnostic::Overfull {
            excess: Scaled::from_raw(shortfall),
        }];
    }
    if glue.badness <= badness_threshold {
        return Vec::new();
    }
    match glue.sign {
        Sign::Stretching => vec![PackDiagnostic::Underfull {
            badness: glue.badness,
            excess: glue.excess,
        }],
        Sign::Shrinking => vec![PackDiagnostic::Tight {
            badness: glue.badness,
            excess: glue.excess,
        }],
        Sign::Normal => Vec::new(),
    }
}

fn measure_hlist(state: &impl TypesetState, nodes: &[Node]) -> Measurement {
    let mut meas = Measurement::ZERO;
    for node in nodes {
        match node {
            Node::Char { font, ch } | Node::Lig { font, ch, .. } => {
                if let Ok(code) = u8::try_from(*ch as u32)
                    && let Some(metrics) = state.font_char_metrics(*font, code)
                {
                    meas.width = add(meas.width, metrics.width);
                    meas.height = meas.height.max(metrics.height);
                    meas.depth = meas.depth.max(metrics.depth);
                }
            }
            Node::Unset => {}
            Node::Kern { amount, .. } => meas.width = add(meas.width, *amount),
            Node::Glue { spec, .. } => add_glue(&mut meas, state.glue(*spec), Axis::Horizontal),
            Node::Rule {
                width,
                height,
                depth,
            } => {
                if let Some(width) = width {
                    meas.width = add(meas.width, *width);
                }
                if let Some(height) = height {
                    meas.height = meas.height.max(*height);
                }
                if let Some(depth) = depth {
                    meas.depth = meas.depth.max(*depth);
                }
            }
            Node::HList(box_node) | Node::VList(box_node) => {
                meas.width = add(meas.width, box_node.width);
                meas.height = meas.height.max(add(box_node.height, box_node.shift));
                meas.depth = meas.depth.max(sub(box_node.depth, box_node.shift));
            }
            Node::Penalty(_) => {}
            Node::Disc { replace, .. } => {
                let replacement = measure_hlist(state, state.nodes(*replace));
                meas.width = add(meas.width, replacement.width);
                meas.height = meas.height.max(replacement.height);
                meas.depth = meas.depth.max(replacement.depth);
            }
            Node::Mark { .. }
            | Node::Ins { .. }
            | Node::Whatsit(_)
            | Node::MathOn
            | Node::MathOff
            | Node::Adjust(_) => {}
        }
    }
    meas
}

fn measure_vlist(state: &impl TypesetState, nodes: &[Node]) -> Measurement {
    let mut meas = Measurement::ZERO;
    for node in nodes {
        match node {
            Node::HList(box_node) | Node::VList(box_node) => {
                meas.height = add(add(meas.height, meas.depth), box_node.height);
                meas.depth = box_node.depth;
                meas.width = meas.width.max(add(box_node.width, box_node.shift));
            }
            Node::Rule {
                width,
                height,
                depth,
            } => {
                meas.height = add(
                    add(meas.height, meas.depth),
                    height.unwrap_or(Scaled::from_raw(0)),
                );
                meas.depth = depth.unwrap_or(Scaled::from_raw(0));
                if let Some(width) = width {
                    meas.width = meas.width.max(*width);
                }
            }
            Node::Kern { amount, .. } => add_vertical_spacing(&mut meas, *amount),
            Node::Glue { spec, .. } => add_glue(&mut meas, state.glue(*spec), Axis::Vertical),
            Node::Penalty(_) => {}
            Node::Char { .. }
            | Node::Lig { .. }
            | Node::Unset
            | Node::Disc { .. }
            | Node::Mark { .. }
            | Node::Ins { .. }
            | Node::Whatsit(_)
            | Node::MathOn
            | Node::MathOff
            | Node::Adjust(_) => {}
        }
    }
    meas
}

#[derive(Clone, Copy)]
enum Axis {
    Horizontal,
    Vertical,
}

fn add_glue(meas: &mut Measurement, spec: GlueSpec, axis: Axis) {
    match axis {
        Axis::Horizontal => meas.width = add(meas.width, spec.width),
        Axis::Vertical => add_vertical_spacing(meas, spec.width),
    }
    meas.stretch[spec.stretch_order as usize] =
        add(meas.stretch[spec.stretch_order as usize], spec.stretch);
    meas.shrink[spec.shrink_order as usize] =
        add(meas.shrink[spec.shrink_order as usize], spec.shrink);
}

fn add_vertical_spacing(meas: &mut Measurement, amount: Scaled) {
    meas.height = add(meas.height, add(meas.depth, amount));
    meas.depth = Scaled::from_raw(0);
}

fn clamp_depth(meas: &mut Measurement, box_max_depth: Scaled) {
    if meas.depth.raw() > box_max_depth.raw() {
        let excess = meas.depth.raw() - box_max_depth.raw();
        meas.height = add(meas.height, Scaled::from_raw(excess));
        meas.depth = box_max_depth;
    }
}

fn vtop_split(
    state: &impl TypesetState,
    list: NodeListId,
    total_height: Scaled,
    total_depth: Scaled,
) -> (Scaled, Scaled) {
    let first = state.nodes(list).iter().find_map(|node| match node {
        Node::HList(box_node) | Node::VList(box_node) => Some((box_node.height, box_node.depth)),
        Node::Rule { height, depth, .. } => Some((
            height.unwrap_or(Scaled::from_raw(0)),
            depth.unwrap_or(Scaled::from_raw(0)),
        )),
        _ => None,
    });
    if let Some((height, depth)) = first {
        let total = total_height.raw().saturating_add(total_depth.raw());
        let new_depth = total.saturating_sub(height.raw()).max(depth.raw());
        (height, Scaled::from_raw(new_depth))
    } else {
        (Scaled::from_raw(0), add(total_height, total_depth))
    }
}

fn add(left: Scaled, right: Scaled) -> Scaled {
    Scaled::from_raw(left.raw().saturating_add(right.raw()))
}

fn sub(left: Scaled, right: Scaled) -> Scaled {
    Scaled::from_raw(left.raw().saturating_sub(right.raw()))
}

#[cfg(test)]
mod tests;
