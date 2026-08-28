//! Execution-side snapshots of packing parameters.

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::node::{Direction, KernKind, Node};
use tex_state::node_arena::PageListId;
use tex_state::scaled::Scaled;
use tex_typeset::{HpackParams, PackSpec, PackedBox, VpackParams};

use crate::pack_report::{
    DiagnosticListLayout, ExecutionDiagnosticContext, PackedDirection, report_pack_diagnostics,
};

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
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    context: &ExecutionDiagnosticContext,
    list: PageListId,
    spec: PackSpec,
    params: HpackParams,
) -> PackedBox {
    let (packed, lr_problems) = hpack_unreported(stores, geometry, list, spec, params);
    report_hpack(stores, diagnostic_effects, context, &packed, lr_problems);
    packed
}

/// Packs a frozen horizontal list without issuing its diagnostics yet.
///
/// TeX's overfull-rule branch mutates the packed list before §663 displays
/// it, so callers that own that branch must finish decoration before calling
/// [`report_hpack`]. Ordinary callers use [`hpack`], which reports once.
pub(crate) fn hpack_unreported<G>(
    stores: &mut CommandContext<'_, G>,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    list: PageListId,
    spec: PackSpec,
    params: HpackParams,
) -> (PackedBox, Option<(usize, usize)>) {
    let (list, lr_problems) = recover_frozen_texxet_directions(stores, list);
    let packed = hpack_prepared_unreported(stores, geometry, list, spec, params);
    (packed, lr_problems)
}

/// Packs a horizontal list whose TeXXeT direction repair has already run.
pub(crate) fn hpack_prepared_unreported<G>(
    stores: &mut CommandContext<'_, G>,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    list: PageListId,
    spec: PackSpec,
    params: HpackParams,
) -> PackedBox {
    let packed = tex_typeset::hpack(
        &crate::typeset_context::TypesetContext::new(stores),
        list,
        spec,
        params,
    );
    stores.set_last_badness(packed.badness);
    geometry.committed_hpack(packed.node.width, packed.node.height, packed.node.depth);
    packed
}

fn recover_frozen_texxet_directions<G>(
    stores: &mut CommandContext<'_, G>,
    list: PageListId,
) -> (PageListId, Option<(usize, usize)>) {
    if stores.int_param(IntParam::TEX_XET_STATE) <= 0 {
        return (list, None);
    }
    let nodes = stores
        .page_node_list(list)
        .expect("packing input belongs to the live page arena")
        .nodes();
    let mut expected = Vec::new();
    let mut extra_indices = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
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
            extra_indices.push(index);
        }
    }
    let source_len = nodes.len();
    let missing = expected.len();
    let extra = extra_indices.len();
    drop(nodes);
    if missing == 0 && extra == 0 {
        return (list, None);
    }

    let mut slices = Vec::new();
    let mut pieces = Vec::with_capacity(extra.saturating_mul(2) + 2);
    let mut start = 0;
    for index in extra_indices {
        if start < index {
            pieces.push(stores.slice_page_node_sequence(list, start..index, &mut slices));
        }
        pieces.push(stores.publish_page_nodes(vec![Node::Kern {
            amount: Scaled::from_raw(0),
            kind: KernKind::Explicit,
        }]));
        start = index + 1;
    }
    if start < source_len {
        pieces.push(stores.slice_page_node_sequence(list, start..source_len, &mut slices));
    }
    if missing != 0 {
        pieces.push(
            stores.publish_page_nodes(expected.into_iter().rev().map(Node::Direction).collect()),
        );
    }
    (
        stores.compose_page_node_sequences(&pieces),
        Some((missing, extra)),
    )
}

pub(crate) fn report_hpack<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    context: &ExecutionDiagnosticContext,
    packed: &PackedBox,
    lr_problems: Option<(usize, usize)>,
) {
    report_pack_diagnostics(
        stores,
        diagnostic_effects,
        context,
        PackedDirection::Horizontal,
        &packed.diagnostics,
        &tex_state::node::Node::HList(packed.node),
        DiagnosticListLayout::FrozenList,
    );
    if let Some((missing, extra)) = lr_problems {
        crate::pack_report::report_lr_problems(
            stores,
            diagnostic_effects,
            context,
            missing,
            extra,
            &Node::HList(packed.node),
            DiagnosticListLayout::FrozenList,
        );
    }
}

pub(crate) fn vpack<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    context: &ExecutionDiagnosticContext,
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
    geometry.committed_vpack(packed.node.width, packed.node.height, packed.node.depth);
    report_pack_diagnostics(
        stores,
        diagnostic_effects,
        context,
        PackedDirection::Vertical,
        &packed.diagnostics,
        &tex_state::node::Node::VList(packed.node),
        DiagnosticListLayout::FrozenList,
    );
    packed
}

pub(crate) fn vtop<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    context: &ExecutionDiagnosticContext,
    list: PageListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    // TeX82 packages the vertical list in §668, including observation-worthy
    // dimensions and diagnostics, before §1087 readjusts the returned vtop.
    let mut packed = vpack(
        stores,
        diagnostic_effects,
        geometry,
        context,
        list,
        spec,
        params,
    );
    let children = packed.node.children;
    tex_typeset::readjust_vtop(
        &crate::typeset_context::TypesetContext::new(stores),
        &children,
        &mut packed,
    );
    packed
}
