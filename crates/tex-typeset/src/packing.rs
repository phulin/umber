use tex_state::glue::Order;
use tex_state::node::Node;
use tex_state::node::{BoxNode, BoxNodeFields, LeaderPayload, Sign, UnsetKind};
use tex_state::node_arena::NodeRef;
use tex_state::node_arena::{NodeCursor, PackedNode, PageListId};
use tex_state::scaled::{GlueSetRatio, Scaled};

#[cfg(test)]
use crate::INF_BAD;
use crate::metrics::{ListMetrics, MetricEvent, MetricOverflow};
use crate::{OVERFULL_BADNESS, TypesetState, badness};

fn add(left: Scaled, right: Scaled) -> Scaled {
    left.checked_add(right)
        .expect("packed dimension overflow must be reported, not saturated")
}

fn sub(left: Scaled, right: Scaled) -> Scaled {
    left.checked_sub(right)
        .expect("packed dimension overflow must be reported, not saturated")
}

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
    pub fn finish(self, children: PageListId) -> PackedBox {
        PackedBox {
            node: BoxNode::new(BoxNodeFields {
                width: self.width,
                height: self.height,
                depth: self.depth,
                shift: Scaled::from_raw(0),
                box_lr: tex_state::node::BoxLr::Normal,
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
pub fn measure_unset(
    state: &impl TypesetState,
    list: &PageListId,
    kind: UnsetKind,
) -> UnsetMetrics {
    let meas = match kind {
        UnsetKind::HBox => measure_hlist(state, state.page_nodes(*list)),
        UnsetKind::VBox => measure_vlist(state, state.page_nodes(*list)),
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
    list: PageListId,
    spec: PackSpec,
    params: HpackParams,
) -> PackedBox {
    let nodes = state.page_nodes(list);
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
            box_lr: tex_state::node::BoxLr::Normal,
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
    list: PageListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    let nodes = state.page_nodes(list);
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
            box_lr: tex_state::node::BoxLr::Normal,
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
    list: PageListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    let mut packed = vpack(state, list, spec, params);
    let children = packed.node.children;
    readjust_vtop(state, &children, &mut packed);
    packed
}

/// Applies TeX82 §1087's post-`vpackage` height/depth adjustment for `\vtop`.
pub fn readjust_vtop(state: &impl TypesetState, list: &PageListId, packed: &mut PackedBox) {
    let (height, depth) = vtop_split(state, list, packed.node.height, packed.node.depth);
    packed.node.height = height;
    packed.node.depth = depth;
}

type Measurement = ListMetrics;

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

fn measure_hlist(state: &impl TypesetState, nodes: NodeCursor<'_>) -> Measurement {
    let mut meas = Measurement::ZERO;
    nodes.for_each(|node| {
        match NodeRef::from(node).packed() {
            PackedNode::Glyph { font, ch } => {
                let metrics = match node {
                    Node::Char { .. } => u8::try_from(ch as u32)
                        .ok()
                        .and_then(|code| {
                            if state.font_uses_tfm_metrics(font) {
                                state.font_characters(font)[usize::from(code)]
                            } else {
                                state.font_character_metrics(font, ch)
                            }
                        })
                        .or_else(|| state.font_character_metrics(font, ch)),
                    _ => state.font_character_metrics(font, ch),
                };
                if let Some(metrics) = metrics {
                    meas.observe_horizontal(
                        MetricEvent::Glyph {
                            width: metrics.width,
                            height: metrics.height,
                            depth: metrics.depth,
                        },
                        MetricOverflow::PACKING,
                    );
                }
            }
            PackedNode::Kern { amount, .. } => {
                meas.observe_horizontal(MetricEvent::Kern(amount), MetricOverflow::PACKING);
            }
            PackedNode::Glue { spec, leader } => {
                meas.observe_horizontal(MetricEvent::Glue(spec), MetricOverflow::PACKING);
                if let Some(leader) = leader {
                    add_hleader_perpendicular_dimensions(&mut meas, leader);
                }
            }
            PackedNode::Rule {
                width,
                height,
                depth,
            } => {
                meas.observe_horizontal(
                    MetricEvent::Rule {
                        width: width.unwrap_or(Scaled::from_raw(0)),
                        height: height.unwrap_or(Scaled::from_raw(0)),
                        depth: depth.unwrap_or(Scaled::from_raw(0)),
                    },
                    MetricOverflow::PACKING,
                );
            }
            PackedNode::Box(box_node) => {
                meas.observe_horizontal(
                    MetricEvent::Box {
                        width: box_node.width,
                        height: box_node.height,
                        depth: box_node.depth,
                        shift: box_node.shift,
                    },
                    MetricOverflow::PACKING,
                );
            }
            PackedNode::Unset(unset) => {
                meas.observe_horizontal(
                    MetricEvent::Box {
                        width: unset.width,
                        height: unset.height,
                        depth: unset.depth,
                        shift: Scaled::from_raw(0),
                    },
                    MetricOverflow::PACKING,
                );
            }
            PackedNode::Image {
                width,
                height,
                depth,
            } => {
                meas.observe_horizontal(
                    MetricEvent::Image {
                        width,
                        height,
                        depth,
                    },
                    MetricOverflow::PACKING,
                );
            }
            PackedNode::Disc(replace) => {
                let replacement = measure_hlist(state, state.page_nodes(replace));
                // A discretionary replacement contributes its natural box
                // dimensions here, but its inner glue is not outer hpack glue.
                meas.merge_horizontal_dimensions(replacement, MetricOverflow::PACKING);
            }
            PackedNode::Math(width) => {
                meas.observe_horizontal(MetricEvent::Math(width), MetricOverflow::PACKING);
            }
            PackedNode::Ignored => {}
        }
    });
    meas
}

fn measure_hlist_nodes(state: &impl TypesetState, nodes: &[Node]) -> Measurement {
    measure_hlist(state, NodeCursor::owned(nodes))
}

fn measure_vlist(_state: &impl TypesetState, nodes: NodeCursor<'_>) -> Measurement {
    let mut meas = Measurement::ZERO;
    nodes.for_each(|node| match NodeRef::from(node).packed() {
        PackedNode::Box(box_node) => {
            meas.observe_vertical(
                MetricEvent::Box {
                    width: box_node.width,
                    height: box_node.height,
                    depth: box_node.depth,
                    shift: box_node.shift,
                },
                MetricOverflow::PACKING,
            );
        }
        PackedNode::Unset(unset) => {
            meas.observe_vertical(
                MetricEvent::Box {
                    width: unset.width,
                    height: unset.height,
                    depth: unset.depth,
                    shift: Scaled::from_raw(0),
                },
                MetricOverflow::PACKING,
            );
        }
        PackedNode::Rule {
            width,
            height,
            depth,
        } => {
            meas.observe_vertical(
                MetricEvent::Rule {
                    width: width.unwrap_or(Scaled::from_raw(0)),
                    height: height.unwrap_or(Scaled::from_raw(0)),
                    depth: depth.unwrap_or(Scaled::from_raw(0)),
                },
                MetricOverflow::PACKING,
            );
        }
        PackedNode::Kern { amount, .. } => {
            meas.observe_vertical(MetricEvent::Kern(amount), MetricOverflow::PACKING);
        }
        PackedNode::Glue { spec, leader } => {
            meas.observe_vertical(MetricEvent::Glue(spec), MetricOverflow::PACKING);
            if let Some(leader) = leader {
                add_vleader_perpendicular_dimensions(&mut meas, leader);
            }
        }
        PackedNode::Image {
            width,
            height,
            depth,
        } => {
            meas.observe_vertical(
                MetricEvent::Image {
                    width,
                    height,
                    depth,
                },
                MetricOverflow::PACKING,
            );
        }
        PackedNode::Glyph { .. }
        | PackedNode::Disc(_)
        | PackedNode::Math(_)
        | PackedNode::Ignored => {}
    });
    meas
}

fn add_hleader_perpendicular_dimensions<List>(
    meas: &mut Measurement,
    leader: &LeaderPayload<List>,
) {
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

fn add_vleader_perpendicular_dimensions<List>(
    meas: &mut Measurement,
    leader: &LeaderPayload<List>,
) {
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

fn clamp_depth(meas: &mut Measurement, box_max_depth: Scaled) {
    if meas.depth.raw() > box_max_depth.raw() {
        let excess = meas.depth.raw() - box_max_depth.raw();
        meas.height = add(meas.height, Scaled::from_raw(excess));
        meas.depth = box_max_depth;
    }
}

fn vtop_split(
    state: &impl TypesetState,
    list: &PageListId,
    total_height: Scaled,
    total_depth: Scaled,
) -> (Scaled, Scaled) {
    let first_height = match state.page_nodes(*list).first() {
        Some(Node::HList(box_node) | Node::VList(box_node)) => box_node.height,
        Some(Node::Rule { height, .. }) => height.unwrap_or(Scaled::from_raw(0)),
        _ => Scaled::from_raw(0),
    };
    let depth = sub(add(total_height, total_depth), first_height);
    (first_height, depth)
}

#[cfg(test)]
mod tests;
