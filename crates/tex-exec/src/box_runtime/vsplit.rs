//! Source-free vertical-box register splitting.

use std::collections::BTreeMap;

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::{DimenParam, GlueParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::Node;
use tex_state::page::PageMark;
use tex_state::scaled::Scaled;
use tex_typeset::{PackSpec, VerticalBreakError, vert_break};

use crate::ExecError;
use crate::diagnostics;
use crate::packing_params::{vpack, vpack_params};
use crate::splitting::{prune_page_top_list_with_discards, vpack_natural};

pub(crate) fn split_vbox_register<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    index: u16,
    height: Scaled,
    error_context: &str,
) -> Result<Option<Node>, ExecError> {
    stores.clear_split_discards();
    let split_top_skip = stores
        .glue_param(GlueParam::SPLIT_TOP_SKIP)
        .map_or(GlueSpec::ZERO, |id| stores.glue(id));
    let split_max_depth = stores.dimen_param(DimenParam::SPLIT_MAX_DEPTH);
    let Some(source) = stores.copy_box_to_page(index) else {
        clear_split_marks(stores);
        return Ok(None);
    };
    let source_node = stores
        .page_node_list(source)
        .expect("copied box belongs to the live page arena")
        .get(0)
        .map(|node| node.to_owned_with(|id| id));
    let Some(source_node) = source_node else {
        clear_split_marks(stores);
        stores.clear_box_preserving_level(index);
        return Ok(None);
    };
    let Node::VList(source_box) = source_node else {
        clear_split_marks(stores);
        let mut report = stores.print_err("");
        report
            .print_esc("vsplit")
            .print(" needs a ")
            .print_esc("vbox")
            .help(&[
                "The box you are trying to split is an \\hbox.",
                "I can't split such a box, so I'll leave it alone.",
            ])
            .context(error_context.to_owned());
        report.error().defer_recovery(diagnostic_effects)?;
        return Ok(None);
    };

    let split_nodes = source_box.children;
    let split = vert_break(
        &crate::typeset_context::TypesetContext::new(stores),
        stores
            .page_node_list(split_nodes)
            .expect("vsplit source belongs to the live page arena")
            .nodes(),
        height,
        split_max_depth,
    )
    .map_err(vertical_break_error)?;
    let split_nodes = normalize_split_infinite_shrink(
        stores,
        split_nodes,
        &split.infinite_shrink_glue,
        diagnostic_context,
        diagnostic_effects,
    )?;
    let (split_list, remainder) = match split.break_index {
        Some(index) => (
            stores.slice_page_node_sequence(split_nodes, 0..index, &mut Vec::new()),
            stores.slice_page_node_sequence(split_nodes, index..split_nodes.len(), &mut Vec::new()),
        ),
        None => (split_nodes, tex_state::node_arena::PageListId::empty()),
    };

    update_split_marks(stores, split_list);
    replace_split_source(
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
        index,
        remainder,
        split_top_skip,
    );

    let mut params = vpack_params(stores);
    params.box_max_depth = split_max_depth;
    Ok(Some(Node::VList(
        vpack(
            stores,
            diagnostic_effects,
            geometry,
            diagnostic_context,
            split_list,
            PackSpec::Exactly(height),
            params,
        )
        .node,
    )))
}

fn normalize_split_infinite_shrink<G>(
    stores: &mut CommandContext<'_, G>,
    nodes: tex_state::node_arena::PageListId,
    indices: &[usize],
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    diagnostic_effects: &mut DiagnosticEffects,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    let mut next_replacement = 0;
    for index in 0..nodes.len() {
        if indices.get(next_replacement).copied() != Some(index) {
            stores.append_page_active_list_range(&mut output, nodes, index..index + 1);
            continue;
        }
        next_replacement += 1;
        let replacement = match stores
            .page_node_list(nodes)
            .expect("vsplit source remains live")
            .nodes()
            .get(index)
        {
            Some(tex_state::node_arena::NodeView::Glue { spec, kind, leader }) => {
                Some((spec, kind, leader))
            }
            _ => None,
        };
        let Some((mut finite, kind, leader)) = replacement else {
            stores.append_page_active_list_range(&mut output, nodes, index..index + 1);
            continue;
        };
        if finite.shrink_order == Order::Normal || finite.shrink.raw() == 0 {
            continue;
        }
        diagnostics::report_split_infinite_shrinkage(
            stores,
            diagnostic_effects,
            diagnostic_context,
        )?;
        finite.shrink_order = Order::Normal;
        stores.push_page_active_list(
            &mut output,
            Node::Glue {
                spec: finite,
                kind,
                leader,
            },
        );
    }
    Ok(stores.finalize_page_active_list(&mut output))
}

fn replace_split_source<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    index: u16,
    remainder: tex_state::node_arena::PageListId,
    split_top_skip: tex_state::glue::GlueSpec,
) {
    let (pruned, discarded) = prune_page_top_list_with_discards(stores, remainder, split_top_skip);
    if stores.int_param(tex_state::env::banks::IntParam::SAVING_V_DISCARDS) > 0 {
        stores.set_split_discards(discarded);
    }
    if pruned.is_empty() {
        stores.clear_box_preserving_level(index);
        return;
    }

    let packed = vpack_natural(
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
        pruned,
    );
    let boxed = stores.construct_page_node(|destination| destination.vlist(packed));
    stores
        .replace_page_box(index, boxed)
        .expect("split remainder stays in admitted page storage");
}

fn update_split_marks<G>(
    stores: &mut CommandContext<'_, G>,
    nodes: tex_state::node_arena::PageListId,
) {
    clear_split_marks(stores);
    let mut classes = BTreeMap::new();
    stores
        .page_node_list(nodes)
        .expect("vsplit prefix remains live")
        .nodes()
        .for_each(|node| {
            if let tex_state::NodeView::Mark { class, tokens } = node {
                let (first, bot) = classes.entry(class).or_insert((None, None));
                if first.is_none() {
                    *first = Some(tokens);
                }
                *bot = Some(tokens);
            }
        });
    for (class, (first, bot)) in classes {
        stores.set_page_mark_class(PageMark::SplitFirst, class, first.unwrap_or_default());
        stores.set_page_mark_class(PageMark::SplitBot, class, bot.unwrap_or_default());
    }
}

fn clear_split_marks<G>(stores: &mut CommandContext<'_, G>) {
    stores.clear_page_mark(PageMark::SplitFirst);
    stores.clear_page_mark(PageMark::SplitBot);
    let classes = stores.page_mark_classes().collect::<Vec<_>>();
    for class in classes {
        stores.clear_page_mark_class(PageMark::SplitFirst, class);
        stores.clear_page_mark_class(PageMark::SplitBot, class);
    }
}

fn vertical_break_error(error: VerticalBreakError) -> ExecError {
    match error {
        VerticalBreakError::ArithmeticOverflow => ExecError::ArithmeticOverflow,
    }
}
