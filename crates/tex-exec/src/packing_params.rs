//! Execution-side snapshots of packing parameters.

use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::ids::NodeListId;
use tex_state::node::{Direction, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::{GeometryObservation, Universe};
use tex_typeset::{HpackParams, PackSpec, PackedBox, VpackParams};

use crate::pack_report::{PackedDirection, report_pack_diagnostics};

#[must_use]
pub(crate) fn hpack_params(stores: &Universe) -> HpackParams {
    HpackParams {
        hbadness: stores.int_param(IntParam::HBADNESS),
        hfuzz: stores.dimen_param(DimenParam::HFUZZ),
        overfull_rule: stores.dimen_param(DimenParam::OVERFULL_RULE),
    }
}

#[must_use]
pub(crate) fn vpack_params(stores: &Universe) -> VpackParams {
    VpackParams {
        vbadness: stores.int_param(IntParam::VBADNESS),
        vfuzz: stores.dimen_param(DimenParam::VFUZZ),
        box_max_depth: stores.dimen_param(DimenParam::BOX_MAX_DEPTH),
    }
}

pub(crate) fn hpack(
    stores: &mut Universe,
    list: NodeListId,
    spec: PackSpec,
    params: HpackParams,
) -> PackedBox {
    let mut recovered = stores.nodes(list).to_vec();
    let lr_problems = recover_texxet_directions(stores, &mut recovered);
    let list = if lr_problems.is_some() {
        stores.freeze_node_list(&recovered)
    } else {
        list
    };
    let packed = tex_typeset::hpack(&*stores, list, spec, params);
    stores.set_last_badness(packed.badness);
    stores.record_geometry_observation(GeometryObservation::Hpack {
        width_sp: i64::from(packed.node.width.raw()),
        height_sp: i64::from(packed.node.height.raw()),
        depth_sp: i64::from(packed.node.depth.raw()),
    });
    report_pack_diagnostics(
        stores,
        PackedDirection::Horizontal,
        &packed.diagnostics,
        &tex_state::node::Node::HList(packed.node),
    );
    if let Some((missing, extra)) = lr_problems {
        crate::pack_report::report_lr_problems(stores, missing, extra, &Node::HList(packed.node));
    }
    packed
}

pub(crate) fn recover_texxet_directions(
    stores: &Universe,
    nodes: &mut Vec<Node>,
) -> Option<(usize, usize)> {
    if stores.int_param(IntParam::TEX_XET_STATE) <= 0 {
        return None;
    }
    let mut expected = Vec::new();
    let mut extra = 0usize;
    for node in nodes.iter_mut() {
        let Node::Direction(direction) = node else {
            continue;
        };
        let closes = match direction {
            Direction::BeginM => Some(Direction::EndM),
            Direction::BeginL => Some(Direction::EndL),
            Direction::BeginR => Some(Direction::EndR),
            Direction::EndM | Direction::EndL | Direction::EndR => None,
        };
        if let Some(closes) = closes {
            expected.push(closes);
        } else if expected.last() == Some(direction) {
            let _ = expected.pop();
        } else {
            *node = Node::Kern {
                amount: Scaled::from_raw(0),
                kind: KernKind::Explicit,
            };
            extra += 1;
        }
    }
    let missing = expected.len();
    nodes.extend(expected.into_iter().rev().map(Node::Direction));
    (missing != 0 || extra != 0).then_some((missing, extra))
}

pub(crate) fn vpack(
    stores: &mut Universe,
    list: NodeListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    let packed = tex_typeset::vpack(&*stores, list, spec, params);
    stores.set_last_badness(packed.badness);
    stores.record_geometry_observation(GeometryObservation::Vpack {
        width_sp: i64::from(packed.node.width.raw()),
        height_sp: i64::from(packed.node.height.raw()),
        depth_sp: i64::from(packed.node.depth.raw()),
    });
    report_pack_diagnostics(
        stores,
        PackedDirection::Vertical,
        &packed.diagnostics,
        &tex_state::node::Node::VList(packed.node),
    );
    packed
}

pub(crate) fn vtop(
    stores: &mut Universe,
    list: NodeListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    let packed = tex_typeset::vtop(&*stores, list, spec, params);
    stores.set_last_badness(packed.badness);
    stores.record_geometry_observation(GeometryObservation::Vpack {
        width_sp: i64::from(packed.node.width.raw()),
        height_sp: i64::from(packed.node.height.raw()),
        depth_sp: i64::from(packed.node.depth.raw()),
    });
    report_pack_diagnostics(
        stores,
        PackedDirection::Vertical,
        &packed.diagnostics,
        &tex_state::node::Node::VList(packed.node),
    );
    packed
}
