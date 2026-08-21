//! Execution-side snapshots of packing parameters.

use tex_state::CommandContext;
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::node::{Direction, KernKind, Node};
use tex_state::node_arena::PageListId;
use tex_state::scaled::Scaled;
use tex_typeset::{HpackParams, PackSpec, PackedBox, VpackParams};

use crate::pack_report::{DiagnosticListLayout, PackedDirection, report_pack_diagnostics};

#[cfg(test)]
mod tests;

#[must_use]
pub(crate) fn hpack_params<G>(stores: &CommandContext<'_, G>) -> HpackParams {
    HpackParams {
        hbadness: stores.int_param(IntParam::HBADNESS),
        hfuzz: stores.dimen_param(DimenParam::HFUZZ),
        overfull_rule: stores.dimen_param(DimenParam::OVERFULL_RULE),
    }
}

#[must_use]
pub(crate) fn vpack_params<G>(stores: &CommandContext<'_, G>) -> VpackParams {
    VpackParams {
        vbadness: stores.int_param(IntParam::VBADNESS),
        vfuzz: stores.dimen_param(DimenParam::VFUZZ),
        box_max_depth: stores.dimen_param(DimenParam::BOX_MAX_DEPTH),
    }
}

pub(crate) fn hpack<G>(
    stores: &mut CommandContext<'_, G>,
    list: PageListId,
    spec: PackSpec,
    params: HpackParams,
) -> PackedBox {
    let (packed, lr_problems) = hpack_unreported(stores, list, spec, params);
    report_hpack(stores, &packed, lr_problems);
    packed
}

/// Packs a frozen horizontal list without issuing its diagnostics yet.
///
/// TeX's overfull-rule branch mutates the packed list before §663 displays
/// it, so callers that own that branch must finish decoration before calling
/// [`report_hpack`]. Ordinary callers use [`hpack`], which reports once.
pub(crate) fn hpack_unreported<G>(
    stores: &mut CommandContext<'_, G>,
    list: PageListId,
    spec: PackSpec,
    params: HpackParams,
) -> (PackedBox, Option<(usize, usize)>) {
    let mut recovered = stores
        .page_node_list(list)
        .expect("packing input belongs to the live page arena")
        .nodes()
        .to_vec();
    let lr_problems = recover_texxet_directions(stores, &mut recovered);
    let list = if lr_problems.is_some() {
        stores.publish_page_nodes(recovered)
    } else {
        list
    };
    let packed = tex_typeset::hpack(
        &crate::typeset_context::TypesetContext::new(stores),
        list,
        spec,
        params,
    );
    stores.set_last_badness(packed.badness);
    (packed, lr_problems)
}

pub(crate) fn report_hpack<G>(
    stores: &mut CommandContext<'_, G>,
    packed: &PackedBox,
    lr_problems: Option<(usize, usize)>,
) {
    report_pack_diagnostics(
        stores,
        PackedDirection::Horizontal,
        &packed.diagnostics,
        &tex_state::node::Node::HList(packed.node.clone()),
        DiagnosticListLayout::FrozenList,
    );
    if let Some((missing, extra)) = lr_problems {
        crate::pack_report::report_lr_problems(
            stores,
            missing,
            extra,
            &Node::HList(packed.node.clone()),
            DiagnosticListLayout::FrozenList,
        );
    }
}

pub(crate) fn recover_texxet_directions<G>(
    stores: &CommandContext<'_, G>,
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

pub(crate) fn vpack<G>(
    stores: &mut CommandContext<'_, G>,
    list: PageListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    let packed = tex_typeset::vpack(
        &crate::typeset_context::TypesetContext::new(stores),
        list,
        spec,
        params,
    );
    stores.set_last_badness(packed.badness);
    report_pack_diagnostics(
        stores,
        PackedDirection::Vertical,
        &packed.diagnostics,
        &tex_state::node::Node::VList(packed.node.clone()),
        DiagnosticListLayout::FrozenList,
    );
    packed
}

pub(crate) fn vtop<G>(
    stores: &mut CommandContext<'_, G>,
    list: PageListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    // TeX82 packages the vertical list in §668, including observation-worthy
    // dimensions and diagnostics, before §1087 readjusts the returned vtop.
    let mut packed = vpack(stores, list, spec, params);
    let children = packed.node.children.clone();
    tex_typeset::readjust_vtop(
        &crate::typeset_context::TypesetContext::new(stores),
        &children,
        &mut packed,
    );
    packed
}
