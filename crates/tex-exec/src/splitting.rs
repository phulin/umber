//! Shared vertical-list splitting helpers for insertions and `\vsplit`.

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::glue::GlueSpec;
use tex_state::node::{BoxNode, GlueKind, Node, Whatsit};
use tex_state::node_arena::{NodeRef, PageListId};
use tex_state::scaled::Scaled;
use tex_typeset::{INF_BAD, PackSpec, VpackParams};

use crate::ExecError;

pub(crate) fn prune_page_top_list<G>(
    stores: &mut CommandContext<'_, G>,
    source: PageListId,
    split_top_skip: GlueSpec,
) -> PageListId {
    let nodes = stores
        .page_node_list(source)
        .expect("page-top source belongs to the live page arena")
        .nodes();
    let mut retained = Vec::<core::ops::Range<usize>>::new();
    let mut run_start = None;
    let mut first_box = None;
    let mut adjusted_top_skip = None;
    for (index, node) in nodes.iter().enumerate() {
        if matches!(node, Node::HList(_) | Node::VList(_) | Node::Rule { .. }) {
            if let Some(start) = run_start.take() {
                retained.push(start..index);
            }
            let adjusted = GlueSpec {
                width: split_top_skip
                    .width
                    .checked_sub(vertical_height(node))
                    .filter(|width| width.raw() > 0)
                    .unwrap_or_else(|| Scaled::from_raw(0)),
                stretch: split_top_skip.stretch,
                stretch_order: split_top_skip.stretch_order,
                shrink: split_top_skip.shrink,
                shrink_order: split_top_skip.shrink_order,
            };
            adjusted_top_skip = Some(adjusted);
            first_box = Some(index);
            break;
        }
        if is_page_top_discardable(node) {
            if let Some(start) = run_start.take() {
                retained.push(start..index);
            }
        } else {
            run_start.get_or_insert(index);
        }
    }
    if first_box.is_none()
        && let Some(start) = run_start
    {
        retained.push(start..nodes.len());
    }
    let source_len = nodes.len();
    let _ = nodes;

    let mut slices = Vec::new();
    let mut pieces = Vec::with_capacity(retained.len() + 2);
    for range in retained {
        pieces.push(stores.slice_page_node_sequence(source, range, &mut slices));
    }
    if let (Some(index), Some(spec)) = (first_box, adjusted_top_skip) {
        pieces.push(stores.publish_page_nodes(vec![Node::Glue {
            spec,
            kind: GlueKind::SplitTopSkip,
            leader: None,
        }]));
        pieces.push(stores.slice_page_node_sequence(source, index..source_len, &mut slices));
    }
    stores.compose_page_node_sequences(&pieces)
}

pub(crate) fn prune_page_top_list_with_discards<G>(
    stores: &mut CommandContext<'_, G>,
    source: PageListId,
    split_top_skip: GlueSpec,
) -> (PageListId, PageListId) {
    let mut retained = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut retained);
    let mut found_box = false;
    for index in 0..source.len() {
        if found_box {
            stores.append_page_active_list_range(&mut retained, source, index..index + 1);
            continue;
        }
        let node = stores
            .page_node_list(source)
            .expect("page-top source belongs to the live page arena")
            .nodes()
            .get(index)
            .expect("page-top source index remains in range");
        match node {
            NodeRef::HList(_) | NodeRef::VList(_) | NodeRef::Rule { .. } => {
                let adjusted = GlueSpec {
                    width: split_top_skip
                        .width
                        .checked_sub(vertical_height_ref(&node))
                        .filter(|width| width.raw() > 0)
                        .unwrap_or_else(|| Scaled::from_raw(0)),
                    stretch: split_top_skip.stretch,
                    stretch_order: split_top_skip.stretch_order,
                    shrink: split_top_skip.shrink,
                    shrink_order: split_top_skip.shrink_order,
                };
                stores.push_page_active_list(
                    &mut retained,
                    Node::Glue {
                        spec: adjusted,
                        kind: GlueKind::SplitTopSkip,
                        leader: None,
                    },
                );
                stores.append_page_active_list_range(&mut retained, source, index..index + 1);
                found_box = true;
            }
            _ if is_page_top_discardable_ref(&node) => {}
            _ => stores.append_page_active_list_range(&mut retained, source, index..index + 1),
        }
    }
    let retained = stores.finalize_page_active_list(&mut retained);

    // The page-material lane owns one persistent builder. The discard prefix
    // is a second coordinate-only projection and therefore starts only after
    // the retained projection has been sealed.
    let mut discarded = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut discarded);
    for index in 0..source.len() {
        let node = stores
            .page_node_list(source)
            .expect("page-top source belongs to the live page arena")
            .nodes()
            .get(index)
            .expect("page-top source index remains in range");
        if matches!(
            node,
            NodeRef::HList(_) | NodeRef::VList(_) | NodeRef::Rule { .. }
        ) {
            break;
        }
        if is_page_top_discardable_ref(&node) {
            stores.append_page_active_list_range(&mut discarded, source, index..index + 1);
        }
    }
    let discarded = stores.finalize_page_active_list(&mut discarded);
    (retained, discarded)
}

/// TeX82 §969's discardable page-top material plus pdfTeX §1378's snap node.
pub(crate) fn is_page_top_discardable(node: &Node) -> bool {
    matches!(
        node,
        Node::Glue { .. }
            | Node::Kern { .. }
            | Node::Penalty(_)
            | Node::Whatsit(Whatsit::PdfSnapY { .. })
    )
}

pub(crate) fn natural_vlist_size<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    content: PageListId,
) -> Result<Scaled, ExecError> {
    let packed = vpack_natural(
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
        content,
    );
    packed
        .height
        .checked_add(packed.depth)
        .ok_or(ExecError::ArithmeticOverflow)
}

pub(crate) fn vpack_natural<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    content: PageListId,
) -> BoxNode {
    crate::packing_params::vpack(
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
        content,
        PackSpec::Natural,
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: Scaled::MAX_DIMEN,
            box_max_depth: Scaled::MAX_DIMEN,
        },
    )
    .node
}

fn vertical_height(node: &Node) -> Scaled {
    NodeRef::from(node)
        .vertical_dimensions()
        .map_or(Scaled::from_raw(0), |(height, _)| height)
}

fn vertical_height_ref(node: &NodeRef<'_>) -> Scaled {
    node.vertical_dimensions()
        .map_or(Scaled::from_raw(0), |(height, _)| height)
}

fn is_page_top_discardable_ref(node: &NodeRef<'_>) -> bool {
    matches!(
        node,
        NodeRef::Glue { .. }
            | NodeRef::Kern { .. }
            | NodeRef::Penalty(_)
            | NodeRef::Whatsit(Whatsit::PdfSnapY { .. })
    )
}

#[cfg(test)]
mod tests;
