//! Pure TeX typesetting kernels.
//!
//! This crate owns list-in/list-out algorithms only. Public packing entry
//! points take immutable state access, copy all parameters into plain structs
//! before doing arithmetic, and never mutate `Universe`.

use tex_state::Universe;
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{FontId, NodeListId};
use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign};
use tex_state::scaled::Scaled;

/// TeX's infinite badness sentinel.
pub const INF_BAD: i32 = 10_000;

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

/// Immutable state access needed by the packing kernels.
pub trait TypesetState {
    fn nodes(&self, id: NodeListId) -> &[Node];
    fn glue(&self, id: tex_state::ids::GlueId) -> GlueSpec;
    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<tex_fonts::CharMetrics>;
}

impl TypesetState for Universe {
    fn nodes(&self, id: NodeListId) -> &[Node] {
        Universe::nodes(self, id)
    }

    fn glue(&self, id: tex_state::ids::GlueId) -> GlueSpec {
        Universe::glue(self, id)
    }

    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<tex_fonts::CharMetrics> {
        Universe::font_char_metrics(self, font, code)
    }
}

/// TeX.web section 108 `badness` function.
#[must_use]
pub fn badness(t: Scaled, s: Scaled) -> i32 {
    let t = t.raw();
    let s = s.raw();
    if t == 0 {
        0
    } else if s <= 0 {
        INF_BAD
    } else {
        let r = if t <= 7_230_584 {
            (t * 297) / s
        } else if s >= 1_663_497 {
            t / (s / 297)
        } else {
            t
        };
        if r > 1290 {
            INF_BAD
        } else {
            ((r * r * r + 0o400000) / 0o1000000).min(INF_BAD)
        }
    }
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
    GlueSetting {
        ratio: if total != 0 {
            f64::from(diff.abs()) / f64::from(total)
        } else {
            0.0
        },
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
                meas.height = meas.height.max(box_node.height);
                meas.depth = meas.depth.max(box_node.depth);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::node::{GlueKind, KernKind};

    fn sp(raw: i32) -> Scaled {
        Scaled::from_raw(raw)
    }

    #[test]
    fn badness_matches_tex_web_boundaries() {
        assert_eq!(badness(sp(0), sp(0)), 0);
        assert_eq!(badness(sp(1), sp(0)), INF_BAD);
        assert_eq!(badness(sp(1), sp(1)), 100);
        assert_eq!(badness(sp(1), sp(4)), 2);
        assert_eq!(badness(sp(1290), sp(297)), 8189);
        assert_eq!(badness(sp(1291), sp(297)), INF_BAD);
        assert_eq!(badness(sp(7_230_584), sp(1)), INF_BAD);
        assert_eq!(badness(sp(7_230_585), sp(1_663_497)), 8189);
    }

    #[test]
    fn hpack_sets_finite_stretch_order_and_ratio() {
        let mut universe = Universe::new();
        let glue = universe.intern_glue(GlueSpec {
            width: sp(10),
            stretch: sp(5),
            stretch_order: Order::Fil,
            shrink: sp(2),
            shrink_order: Order::Normal,
        });
        let list = universe.freeze_node_list(&[
            Node::Kern {
                amount: sp(20),
                kind: KernKind::Explicit,
            },
            Node::Glue {
                spec: glue,
                kind: GlueKind::Normal,
            },
        ]);
        let packed = hpack(
            &universe,
            list,
            PackSpec::Exactly(sp(40)),
            HpackParams {
                hbadness: INF_BAD,
                hfuzz: sp(0),
                overfull_rule: sp(0),
            },
        );
        assert_eq!(packed.node.width, sp(40));
        assert_eq!(packed.node.glue_sign, Sign::Stretching);
        assert_eq!(packed.node.glue_order, Order::Fil);
        assert_eq!(packed.node.glue_set, 2.0);
    }

    #[test]
    fn vpack_clamps_depth_to_box_max_depth() {
        let mut universe = Universe::new();
        let child = universe.freeze_node_list(&[]);
        let list = universe.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: sp(5),
            height: sp(10),
            depth: sp(8),
            shift: sp(0),
            glue_set: 0.0,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: child,
        }))]);
        let packed = vpack(
            &universe,
            list,
            PackSpec::Natural,
            VpackParams {
                vbadness: INF_BAD,
                vfuzz: sp(0),
                box_max_depth: sp(3),
            },
        );
        assert_eq!(packed.node.height, sp(15));
        assert_eq!(packed.node.depth, sp(3));
    }

    #[test]
    fn vertical_spacing_consumes_previous_depth() {
        let mut universe = Universe::new();
        let child = universe.freeze_node_list(&[]);
        let glue = universe.intern_glue(GlueSpec {
            width: sp(7),
            stretch: sp(0),
            stretch_order: Order::Normal,
            shrink: sp(0),
            shrink_order: Order::Normal,
        });
        let hbox = Node::HList(BoxNode::new(BoxNodeFields {
            width: sp(6),
            height: sp(4),
            depth: sp(1),
            shift: sp(0),
            glue_set: 0.0,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: child,
        }));
        let list = universe.freeze_node_list(&[
            hbox.clone(),
            Node::Glue {
                spec: glue,
                kind: GlueKind::BaselineSkip,
            },
            hbox,
        ]);
        let packed = vpack(
            &universe,
            list,
            PackSpec::Natural,
            VpackParams {
                vbadness: INF_BAD,
                vfuzz: sp(0),
                box_max_depth: sp(100),
            },
        );
        assert_eq!(packed.node.height, sp(16));
        assert_eq!(packed.node.depth, sp(1));
    }

    #[test]
    fn packed_box_can_round_trip_through_survivor_box_register() {
        let mut universe = Universe::new();
        let list = universe.freeze_node_list(&[Node::Kern {
            amount: sp(12),
            kind: KernKind::Explicit,
        }]);
        let packed = hpack(
            &universe,
            list,
            PackSpec::Natural,
            HpackParams {
                hbadness: INF_BAD,
                hfuzz: sp(0),
                overfull_rule: sp(0),
            },
        );
        let boxed = universe.freeze_node_list(&[Node::HList(packed.node)]);
        universe.set_box_reg(0, boxed);
        let survivor = universe.box_reg(0).expect("box should be stored");
        assert!(matches!(universe.nodes(survivor), [Node::HList(_)]));
    }
}
