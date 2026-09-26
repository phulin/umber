//! Shared vertical-list splitting helpers for insertions and `\vsplit`.

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::glue::GlueSpec;
use tex_state::node::{BoxNode, GlueKind, Node, Whatsit};
use tex_state::node_view::NodeView;
use tex_state::page_node_arena::PageListId;
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
    let stopped = nodes.try_for_each_range(0..nodes.len(), |index, node| {
        if matches!(
            node,
            tex_state::NodeView::HList(_)
                | tex_state::NodeView::VList(_)
                | tex_state::NodeView::Rule { .. }
        ) {
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
            return core::ops::ControlFlow::Break(());
        }
        if is_page_top_discardable(node) {
            if let Some(start) = run_start.take() {
                retained.push(start..index);
            }
        } else {
            run_start.get_or_insert(index);
        }
        core::ops::ControlFlow::Continue(())
    });
    debug_assert_eq!(stopped.is_break(), first_box.is_some());
    if first_box.is_none()
        && let Some(start) = run_start
    {
        retained.push(start..nodes.len());
    }
    let source_len = nodes.len();
    let _ = nodes;

    let mut pieces = Vec::with_capacity(retained.len() + 2);
    for range in retained {
        pieces.push(stores.slice_page_node_sequence(source, range));
    }
    if let (Some(index), Some(spec)) = (first_box, adjusted_top_skip) {
        pieces.push(stores.construct_page_node(|destination| {
            destination.glue(
                spec,
                GlueKind::SplitTopSkip,
                tex_state::node::GlueSpecOrigin::Owned,
                None,
            );
        }));
        pieces.push(stores.slice_page_node_sequence(source, index..source_len));
    }
    stores.compose_page_node_sequences(&pieces)
}

pub(crate) fn prune_page_top_list_with_discards<G>(
    stores: &mut CommandContext<'_, G>,
    source: PageListId,
    split_top_skip: GlueSpec,
) -> (PageListId, PageListId) {
    let nodes = stores
        .page_node_list(source)
        .expect("page-top source belongs to the live page arena")
        .nodes();
    let mut retained_ranges = Vec::<core::ops::Range<usize>>::new();
    let mut discarded_ranges = Vec::<core::ops::Range<usize>>::new();
    let mut retained_start = None;
    let mut discarded_start = None;
    let mut first_box = None;
    let mut adjusted_top_skip = None;
    let stopped = nodes.try_for_each_range(0..nodes.len(), |index, node| {
        if matches!(
            node,
            NodeView::HList(_) | NodeView::VList(_) | NodeView::Rule { .. }
        ) {
            if let Some(start) = retained_start.take() {
                retained_ranges.push(start..index);
            }
            if let Some(start) = discarded_start.take() {
                discarded_ranges.push(start..index);
            }
            adjusted_top_skip = Some(GlueSpec {
                width: split_top_skip
                    .width
                    .checked_sub(vertical_height(node))
                    .filter(|width| width.raw() > 0)
                    .unwrap_or_else(|| Scaled::from_raw(0)),
                stretch: split_top_skip.stretch,
                stretch_order: split_top_skip.stretch_order,
                shrink: split_top_skip.shrink,
                shrink_order: split_top_skip.shrink_order,
            });
            first_box = Some(index);
            return core::ops::ControlFlow::Break(());
        }
        if is_page_top_discardable(node) {
            if let Some(start) = retained_start.take() {
                retained_ranges.push(start..index);
            }
            discarded_start.get_or_insert(index);
        } else {
            if let Some(start) = discarded_start.take() {
                discarded_ranges.push(start..index);
            }
            retained_start.get_or_insert(index);
        }
        core::ops::ControlFlow::Continue(())
    });
    debug_assert_eq!(stopped.is_break(), first_box.is_some());
    if first_box.is_none() {
        if let Some(start) = retained_start {
            retained_ranges.push(start..nodes.len());
        }
        if let Some(start) = discarded_start {
            discarded_ranges.push(start..nodes.len());
        }
    }
    let source_len = nodes.len();
    let _ = nodes;

    let mut retained = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut retained);
    for range in retained_ranges {
        stores.append_page_active_list_range(&mut retained, source, range);
    }
    if let (Some(index), Some(spec)) = (first_box, adjusted_top_skip) {
        stores.push_page_active_list(
            &mut retained,
            Node::Glue {
                origin: tex_state::node::GlueSpecOrigin::Owned,
                spec,
                kind: GlueKind::SplitTopSkip,
                leader: None,
            },
        );
        stores.append_page_active_list_range(&mut retained, source, index..source_len);
    }
    let retained = stores.finalize_page_active_list(&mut retained);

    // The page-material lane owns one persistent builder. The discard prefix
    // is a second coordinate-only projection and therefore starts only after
    // the retained projection has been sealed.
    let mut discarded = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut discarded);
    for range in discarded_ranges {
        stores.append_page_active_list_range(&mut discarded, source, range);
    }
    let discarded = stores.finalize_page_active_list(&mut discarded);
    (retained, discarded)
}

/// TeX82 §969's discardable page-top material plus pdfTeX §1378's snap node.
pub(crate) fn is_page_top_discardable(node: NodeView<'_>) -> bool {
    matches!(
        node,
        NodeView::Glue { .. }
            | NodeView::Kern { .. }
            | NodeView::Penalty(_)
            | NodeView::Whatsit(Whatsit::PdfSnapY { .. })
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
        &diagnostic_context.packing(),
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

fn vertical_height(node: NodeView<'_>) -> Scaled {
    node.vertical_dimensions()
        .map_or(Scaled::from_raw(0), |(height, _)| height)
}

#[cfg(test)]
mod tests;
