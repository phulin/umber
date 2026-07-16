use tex_arith::{saturating_add as add, saturating_sub as sub};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::NodeListId;
use tex_state::node::Node;
use tex_state::node::{BoxNode, BoxNodeFields, LeaderPayload, Sign, UnsetKind};
use tex_state::node_arena::{NodeList, NodeRef};
use tex_state::scaled::{GlueSetRatio, Scaled};

#[cfg(test)]
use crate::INF_BAD;
use crate::{OVERFULL_BADNESS, TypesetState, badness};

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

/// Parameters used by vertical packing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VpackParams {
    pub vbadness: i32,
    pub vfuzz: Scaled,
    pub box_max_depth: Scaled,
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
    pub badness: i32,
    pub diagnostics: Vec<PackDiagnostic>,
}

/// Horizontal packing result whose decoded children have not yet been frozen.
///
/// This lets construction code measure an owned list directly, append any
/// overfull rule, and freeze the final children only once.
#[derive(Clone, Debug, PartialEq)]
pub struct HpackPlan {
    width: Scaled,
    height: Scaled,
    depth: Scaled,
    glue: GlueSetting,
    pub diagnostics: Vec<PackDiagnostic>,
}

impl HpackPlan {
    #[must_use]
    pub fn finish(self, children: NodeListId) -> PackedBox {
        PackedBox {
            node: BoxNode::new(BoxNodeFields {
                width: self.width,
                height: self.height,
                depth: self.depth,
                shift: Scaled::from_raw(0),
                display: false,
                glue_set: self.glue.ratio,
                glue_sign: self.glue.sign,
                glue_order: self.glue.order,
                children,
            }),
            badness: self.glue.badness,
            diagnostics: self.diagnostics,
        }
    }
}

/// Natural dimensions and glue totals for an unset alignment box.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnsetMetrics {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub stretch: Scaled,
    pub stretch_order: Order,
    pub shrink: Scaled,
    pub shrink_order: Order,
}

#[must_use]
pub fn measure_unset(state: &impl TypesetState, list: NodeListId, kind: UnsetKind) -> UnsetMetrics {
    let meas = match kind {
        UnsetKind::HBox => measure_hlist(state, state.nodes(list)),
        UnsetKind::VBox => measure_vlist(state, state.nodes(list)),
    };
    let stretch_order = highest_order(meas.stretch);
    let shrink_order = highest_order(meas.shrink);
    UnsetMetrics {
        width: meas.width,
        height: meas.height,
        depth: meas.depth,
        stretch: meas.stretch[stretch_order as usize],
        stretch_order,
        shrink: meas.shrink[shrink_order as usize],
        shrink_order,
    }
}

#[must_use]
pub fn hpack(
    state: &impl TypesetState,
    list: NodeListId,
    spec: PackSpec,
    params: HpackParams,
) -> PackedBox {
    let nodes = state.nodes(list);
    let has_content = !nodes.is_empty();
    let meas = measure_hlist(state, nodes);
    let width = target_size(meas.width, spec);
    let glue = set_glue(width, meas.width, &meas, has_content);
    let diagnostics = hpack_diagnostics(glue, params);
    PackedBox {
        node: BoxNode::new(BoxNodeFields {
            width,
            height: meas.height,
            depth: meas.depth,
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: glue.ratio,
            glue_sign: glue.sign,
            glue_order: glue.order,
            children: list,
        }),
        badness: glue.badness,
        diagnostics,
    }
}

/// Plans an hbox directly from decoded construction nodes.
#[must_use]
pub fn plan_hpack_nodes(
    state: &impl TypesetState,
    nodes: &[Node],
    spec: PackSpec,
    params: HpackParams,
) -> HpackPlan {
    let meas = measure_hlist_nodes(state, nodes);
    let width = target_size(meas.width, spec);
    let glue = set_glue(width, meas.width, &meas, !nodes.is_empty());
    HpackPlan {
        width,
        height: meas.height,
        depth: meas.depth,
        glue,
        diagnostics: hpack_diagnostics(glue, params),
    }
}

#[must_use]
pub fn vpack(
    state: &impl TypesetState,
    list: NodeListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    let nodes = state.nodes(list);
    let has_content = !nodes.is_empty();
    let mut meas = measure_vlist(state, nodes);
    clamp_depth(&mut meas, params.box_max_depth);
    let height = target_size(meas.height, spec);
    let glue = set_glue(height, meas.height, &meas, has_content);
    let diagnostics = vpack_diagnostics(glue, params);
    PackedBox {
        node: BoxNode::new(BoxNodeFields {
            width: meas.width,
            height,
            depth: meas.depth,
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: glue.ratio,
            glue_sign: glue.sign,
            glue_order: glue.order,
            children: list,
        }),
        badness: glue.badness,
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
    has_glue: bool,
}

impl Measurement {
    const ZERO: Self = Self {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        stretch: [Scaled::from_raw(0); 4],
        shrink: [Scaled::from_raw(0); 4],
        has_glue: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct GlueSetting {
    ratio: GlueSetRatio,
    sign: Sign,
    order: Order,
    badness: i32,
    excess: Scaled,
    overfull_excess: Scaled,
}

fn target_size(natural: Scaled, spec: PackSpec) -> Scaled {
    match spec {
        PackSpec::Natural => natural,
        PackSpec::Exactly(size) => size,
        PackSpec::Spread(extra) => add(natural, extra),
    }
}

fn set_glue(target: Scaled, natural: Scaled, meas: &Measurement, has_content: bool) -> GlueSetting {
    let diff = target.raw() - natural.raw();
    if diff == 0 || !has_content {
        return GlueSetting {
            ratio: GlueSetRatio::ZERO,
            sign: Sign::Normal,
            order: Order::Normal,
            badness: 0,
            excess: Scaled::from_raw(0),
            overfull_excess: Scaled::from_raw(0),
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
        GlueSetRatio::ZERO
    } else if sign == Sign::Shrinking && order == Order::Normal && excess.raw() > total {
        GlueSetRatio::UNITY
    } else {
        GlueSetRatio::from_scaled_ratio(excess, Scaled::from_raw(total))
    };
    let overfull_excess =
        if sign == Sign::Shrinking && order == Order::Normal && excess.raw() > total {
            sub(excess, Scaled::from_raw(total))
        } else {
            Scaled::from_raw(0)
        };
    GlueSetting {
        ratio,
        sign,
        order,
        badness: if overfull_excess.raw() > 0 {
            // TeX.web §§664 and 676 reserve 1000000 for a nonempty
            // box whose normal-order glue cannot shrink far enough.
            OVERFULL_BADNESS
        } else if order == Order::Normal {
            badness(excess, Scaled::from_raw(total))
        } else {
            0
        },
        excess,
        overfull_excess,
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

fn hpack_diagnostics(glue: GlueSetting, params: HpackParams) -> Vec<PackDiagnostic> {
    common_diagnostics(glue, params.hbadness, params.hfuzz)
}

fn vpack_diagnostics(glue: GlueSetting, params: VpackParams) -> Vec<PackDiagnostic> {
    common_diagnostics(glue, params.vbadness, params.vfuzz)
}

fn common_diagnostics(
    glue: GlueSetting,
    badness_threshold: i32,
    fuzz: Scaled,
) -> Vec<PackDiagnostic> {
    if glue.overfull_excess.raw() > 0 {
        return if glue.overfull_excess.raw() > fuzz.raw() || badness_threshold < 100 {
            vec![PackDiagnostic::Overfull {
                excess: glue.overfull_excess,
            }]
        } else {
            Vec::new()
        };
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

fn measure_hlist(state: &impl TypesetState, nodes: NodeList<'_>) -> Measurement {
    let mut meas = Measurement::ZERO;
    let mut index = 0;
    while index < nodes.len() {
        if let Some(run) = nodes.char_codes(index) {
            let font = run.font();
            let widths = state.font_widths(font);
            let characters = state.font_characters(font);
            let mut run_len = 0;
            for code in run {
                // Keep TeX's saturating additions in source order. The compact
                // run removes tag/font dispatch without changing overflow.
                meas.width = add(meas.width, widths[usize::from(code)]);
                if let Some(metrics) = characters.get(usize::from(code)).copied().flatten() {
                    meas.height = meas.height.max(metrics.height);
                    meas.depth = meas.depth.max(metrics.depth);
                }
                run_len += 1;
            }
            index += run_len;
            continue;
        }
        let node = nodes.get(index).expect("index is within node list");
        match node {
            NodeRef::Char { font, ch, .. } | NodeRef::Lig { font, ch, .. } => {
                if let Ok(code) = u8::try_from(ch as u32)
                    && let Some(metrics) = state.font_char_metrics(font, code)
                {
                    meas.width = add(meas.width, metrics.width);
                    meas.height = meas.height.max(metrics.height);
                    meas.depth = meas.depth.max(metrics.depth);
                }
            }
            NodeRef::Kern { amount, .. } => meas.width = add(meas.width, amount),
            NodeRef::Glue { spec, leader, .. } => {
                add_glue(&mut meas, state.glue(spec), Axis::Horizontal);
                if let Some(leader) = leader {
                    add_hleader_perpendicular_dimensions(&mut meas, leader);
                }
            }
            NodeRef::Rule {
                width,
                height,
                depth,
            } => {
                if let Some(width) = width {
                    meas.width = add(meas.width, width);
                }
                if let Some(height) = height {
                    meas.height = meas.height.max(height);
                }
                if let Some(depth) = depth {
                    meas.depth = meas.depth.max(depth);
                }
            }
            NodeRef::HList(box_node) | NodeRef::VList(box_node) => {
                meas.width = add(meas.width, box_node.width);
                meas.height = meas.height.max(sub(box_node.height, box_node.shift));
                meas.depth = meas.depth.max(add(box_node.depth, box_node.shift));
            }
            NodeRef::Unset(unset) => {
                meas.width = add(meas.width, unset.width);
                meas.height = meas.height.max(unset.height);
                meas.depth = meas.depth.max(unset.depth);
            }
            NodeRef::Penalty(_) => {}
            NodeRef::Disc { replace, .. } => {
                let replacement = measure_hlist(state, state.nodes(replace));
                meas.width = add(meas.width, replacement.width);
                meas.height = meas.height.max(replacement.height);
                meas.depth = meas.depth.max(replacement.depth);
                meas.has_glue |= replacement.has_glue;
            }
            NodeRef::Mark { .. }
            | NodeRef::Ins { .. }
            | NodeRef::Whatsit(_)
            | NodeRef::MathNoad(_)
            | NodeRef::FractionNoad(_)
            | NodeRef::MathStyle(_)
            | NodeRef::MathChoice(_)
            | NodeRef::MathList(_)
            | NodeRef::Nonscript
            | NodeRef::Direction(_)
            | NodeRef::Adjust(_) => {}
            NodeRef::MathOn(width) | NodeRef::MathOff(width) => {
                meas.width = add(meas.width, width);
            }
        }
        index += 1;
    }
    meas
}

fn measure_hlist_nodes(state: &impl TypesetState, nodes: &[Node]) -> Measurement {
    let mut meas = Measurement::ZERO;
    for node in nodes {
        match node {
            Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => {
                if let Ok(code) = u8::try_from(*ch as u32)
                    && let Some(metrics) = state.font_char_metrics(*font, code)
                {
                    meas.width = add(meas.width, metrics.width);
                    meas.height = meas.height.max(metrics.height);
                    meas.depth = meas.depth.max(metrics.depth);
                }
            }
            Node::Kern { amount, .. } => meas.width = add(meas.width, *amount),
            Node::Glue { spec, leader, .. } => {
                add_glue(&mut meas, state.glue(*spec), Axis::Horizontal);
                if let Some(leader) = leader {
                    add_hleader_perpendicular_dimensions(&mut meas, leader);
                }
            }
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
                meas.height = meas.height.max(sub(box_node.height, box_node.shift));
                meas.depth = meas.depth.max(add(box_node.depth, box_node.shift));
            }
            Node::Unset(unset) => {
                meas.width = add(meas.width, unset.width);
                meas.height = meas.height.max(unset.height);
                meas.depth = meas.depth.max(unset.depth);
            }
            Node::Disc { replace, .. } => {
                let replacement = measure_hlist(state, state.nodes(*replace));
                meas.width = add(meas.width, replacement.width);
                meas.height = meas.height.max(replacement.height);
                meas.depth = meas.depth.max(replacement.depth);
                meas.has_glue |= replacement.has_glue;
            }
            Node::MathOn(width) | Node::MathOff(width) => {
                meas.width = add(meas.width, *width);
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
    }
    meas
}

fn measure_vlist(state: &impl TypesetState, nodes: NodeList<'_>) -> Measurement {
    let mut meas = Measurement::ZERO;
    for node in nodes {
        match node {
            NodeRef::HList(box_node) | NodeRef::VList(box_node) => {
                meas.height = add(add(meas.height, meas.depth), box_node.height);
                meas.depth = box_node.depth;
                meas.width = meas.width.max(add(box_node.width, box_node.shift));
            }
            NodeRef::Unset(unset) => {
                meas.height = add(add(meas.height, meas.depth), unset.height);
                meas.depth = unset.depth;
                meas.width = meas.width.max(unset.width);
            }
            NodeRef::Rule {
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
                    meas.width = meas.width.max(width);
                }
            }
            NodeRef::Kern { amount, .. } => add_vertical_spacing(&mut meas, amount),
            NodeRef::Glue { spec, leader, .. } => {
                add_glue(&mut meas, state.glue(spec), Axis::Vertical);
                if let Some(leader) = leader {
                    add_vleader_perpendicular_dimensions(&mut meas, leader);
                }
            }
            NodeRef::Penalty(_) => {}
            NodeRef::Char { .. }
            | NodeRef::Lig { .. }
            | NodeRef::Disc { .. }
            | NodeRef::Mark { .. }
            | NodeRef::Ins { .. }
            | NodeRef::Whatsit(_)
            | NodeRef::MathOn(_)
            | NodeRef::MathOff(_)
            | NodeRef::Direction(_)
            | NodeRef::MathNoad(_)
            | NodeRef::FractionNoad(_)
            | NodeRef::MathStyle(_)
            | NodeRef::MathChoice(_)
            | NodeRef::MathList(_)
            | NodeRef::Nonscript
            | NodeRef::Adjust(_) => {}
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
    meas.has_glue = true;
    match axis {
        Axis::Horizontal => meas.width = add(meas.width, spec.width),
        Axis::Vertical => add_vertical_spacing(meas, spec.width),
    }
    meas.stretch[spec.stretch_order as usize] =
        add(meas.stretch[spec.stretch_order as usize], spec.stretch);
    meas.shrink[spec.shrink_order as usize] =
        add(meas.shrink[spec.shrink_order as usize], spec.shrink);
}

fn add_hleader_perpendicular_dimensions(meas: &mut Measurement, leader: &LeaderPayload) {
    match leader {
        LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node) => {
            meas.height = meas.height.max(box_node.height);
            meas.depth = meas.depth.max(box_node.depth);
        }
        LeaderPayload::Rule { height, depth, .. } => {
            if let Some(height) = height {
                meas.height = meas.height.max(*height);
            }
            if let Some(depth) = depth {
                meas.depth = meas.depth.max(*depth);
            }
        }
    }
}

fn add_vleader_perpendicular_dimensions(meas: &mut Measurement, leader: &LeaderPayload) {
    match leader {
        LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node) => {
            meas.width = meas.width.max(box_node.width);
        }
        LeaderPayload::Rule { width, .. } => {
            if let Some(width) = width {
                meas.width = meas.width.max(*width);
            }
        }
    }
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
    let first_height = match state.nodes(list).first() {
        Some(NodeRef::HList(box_node) | NodeRef::VList(box_node)) => box_node.height,
        Some(NodeRef::Rule { height, .. }) => height.unwrap_or(Scaled::from_raw(0)),
        _ => Scaled::from_raw(0),
    };
    let depth = sub(add(total_height, total_depth), first_height);
    (first_height, depth)
}

#[cfg(test)]
mod tests;
