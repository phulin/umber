//! Source-free horizontal packing and box-register lookup.

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::node::Node;
use tex_state::node_arena::PageListId;
use tex_typeset::{PackDiagnostic, PackSpec};

use crate::packing_params::hpack_params;
use crate::{ExecError, Mode, ModeNest};

use super::hmode::flush_pending_hchars;

pub(crate) fn take_last_box<G, F>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    error_context: F,
) -> Result<Option<Node>, ExecError>
where
    F: FnOnce(&CommandContext<'_, G>) -> Result<String, ExecError>,
{
    flush_pending_hchars(nest, stores, diagnostic_effects, fuel)?;
    match nest.current_mode() {
        Mode::Math | Mode::DisplayMath => {
            let error_context = error_context(stores)?;
            report_cannot_take_last_box(
                stores,
                diagnostic_effects,
                "math mode",
                &["Sorry; this \\lastbox will be void."],
                error_context,
            )?;
            Ok(None)
        }
        Mode::Vertical
            if nest.current_list().is_empty() && stores.page_contributions().is_empty() =>
        {
            let error_context = error_context(stores)?;
            report_cannot_take_last_box(
                stores,
                diagnostic_effects,
                "vertical mode",
                &[
                    "Sorry...I usually can't take things from the current page.",
                    "This \\lastbox will therefore be void.",
                ],
                error_context,
            )?;
            Ok(None)
        }
        Mode::Vertical => {
            let Some(tail) =
                crate::effective_tail::EffectiveTail::find(stores.page_contributions().iter())
            else {
                return Ok(None);
            };
            if !matches!(tail.node(), Node::HList(_) | Node::VList(_)) {
                return Ok(None);
            }
            let removed = stores.remove_page_contribution_range(tail.removal_range());
            let result = reset_removed_box_shift(stores.page_carrier_node(&removed));
            stores.discard_page_node(removed);
            Ok(result)
        }
        Mode::InternalVertical | Mode::Horizontal | Mode::RestrictedHorizontal => {
            let current_list = nest.current_list();
            let Some(tail) =
                crate::effective_tail::EffectiveTail::find(current_list.nodes(stores).iter())
            else {
                return Ok(None);
            };
            if !matches!(tail.node(), Node::HList(_) | Node::VList(_)) {
                return Ok(None);
            }
            let range = tail.removal_range();
            let _ = current_list;
            let removed = nest
                .current_list_mutation()
                .remove_node_range(stores, range);
            let node = stores
                .page_node_list(removed)
                .expect("removed last-box range belongs to the live page arena")
                .nodes()
                .first()
                .expect("effective-tail removal contains its box node");
            Ok(reset_removed_box_shift(node))
        }
    }
}

fn reset_removed_box_shift(node: &Node) -> Option<Node> {
    let mut node = node.clone();
    match &mut node {
        Node::HList(box_node) | Node::VList(box_node) => {
            box_node.shift = tex_state::scaled::Scaled::from_raw(0);
            Some(node)
        }
        _ => None,
    }
}

fn report_cannot_take_last_box<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    mode: &str,
    help: &[&str],
    context: String,
) -> Result<(), ExecError> {
    let mut report = stores.print_err("You can't use `");
    report
        .print_esc("lastbox")
        .print("' in ")
        .print(mode)
        .help(help)
        .context(context);
    report.error().defer_recovery(diagnostic_effects)?;
    Ok(())
}

pub(crate) fn hpack_with_overfull_rule<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    context: &crate::pack_report::ExecutionDiagnosticContext,
    children: tex_state::node_arena::PageListId,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = hpack_params(stores);
    let (mut packed, lr_problems) =
        crate::packing_params::hpack_unreported(stores, geometry, children, spec, params);
    if !packed.node.children.is_empty()
        && params.overfull_rule.raw() > 0
        && packed
            .diagnostics
            .iter()
            .any(|diagnostic| {
                matches!(diagnostic, PackDiagnostic::Overfull { excess } if *excess > params.hfuzz)
            })
    {
        let overfull_rule = stores.publish_page_nodes(vec![Node::Rule {
            width: Some(params.overfull_rule),
            height: None,
            depth: None,
        }]);
        packed.node.children = stores
            .compose_page_node_sequences(&[packed.node.children, overfull_rule]);
    }
    // TeX82 §§115/162 stores a discretionary's replacement as the
    // physical nodes immediately following the disc node. Umber keeps that
    // material in `replace` so semantic list traversal cannot count it twice;
    // retain the physical projection separately for §182's diagnostic walk.
    if let Some(diagnostic_children) =
        physical_discretionary_projection(stores, packed.node.children)
    {
        packed.node.diagnostic_children = Some(diagnostic_children);
    }
    crate::packing_params::report_hpack(stores, diagnostic_effects, context, &packed, lr_problems);
    packed.node
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn hpack_page_list_with_diagnostics<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    context: &crate::pack_report::ExecutionDiagnosticContext,
    children: PageListId,
    diagnostic_children: Option<PageListId>,
    direction_scratch: &mut Vec<tex_state::node::Direction>,
    allocator_high_cell_overlap: u32,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = hpack_params(stores);
    let (children, lr_problems) =
        recover_texxet_directions_list(stores, children, direction_scratch);
    let (mut diagnostic_children, _) = diagnostic_children
        .map(|nodes| recover_texxet_directions_list(stores, nodes, direction_scratch))
        .unzip();
    let mut packed =
        crate::packing_params::hpack_prepared_unreported(stores, geometry, children, spec, params);
    if !packed.node.children.is_empty()
        && params.overfull_rule.raw() > 0
        && packed.diagnostics.iter().any(|diagnostic| {
            matches!(diagnostic, PackDiagnostic::Overfull { excess } if *excess > params.hfuzz)
        })
    {
        let mut generated = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
        stores.open_page_active_list(&mut generated);
        stores.push_page_active_list(
            &mut generated,
            Node::Rule {
                width: Some(params.overfull_rule),
                height: None,
                depth: None,
            },
        );
        let generated = stores.finalize_page_active_list(&mut generated);
        let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
        stores.open_page_active_list(&mut output);
        stores.append_page_active_list(&mut output, packed.node.children);
        stores.append_page_active_list(&mut output, generated);
        packed.node.children = stores.finalize_page_active_list(&mut output);
        if let Some(physical) = diagnostic_children {
            let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
            stores.open_page_active_list(&mut output);
            stores.append_page_active_list(&mut output, physical);
            stores.append_page_active_list(&mut output, generated);
            diagnostic_children = Some(stores.finalize_page_active_list(&mut output));
        }
    }
    let short_diagnostic_children = diagnostic_children
        .map(|physical| project_short_diagnostic_discs_list(stores, physical, children));
    let diagnostic_list_layout = if short_diagnostic_children.is_some() {
        crate::pack_report::DiagnosticListLayout::DetachedProjection
    } else {
        crate::pack_report::DiagnosticListLayout::FrozenList
    };
    packed.node.allocator_high_cell_overlap = if diagnostic_children.is_some() {
        allocator_high_cell_overlap
    } else {
        0
    };
    packed.node.diagnostic_children = diagnostic_children;
    stores.set_last_badness(packed.badness);
    let diagnostic_box = tex_state::node::BoxNode {
        children: short_diagnostic_children.unwrap_or(packed.node.children),
        ..packed.node
    };
    crate::pack_report::report_pack_diagnostics(
        stores,
        diagnostic_effects,
        context,
        crate::pack_report::PackedDirection::Horizontal,
        &packed.diagnostics,
        &Node::HList(diagnostic_box),
        diagnostic_list_layout,
    );
    if let Some((missing, extra)) = lr_problems {
        crate::pack_report::report_lr_problems(
            stores,
            diagnostic_effects,
            context,
            missing,
            extra,
            &Node::HList(diagnostic_box),
            diagnostic_list_layout,
        );
    }
    packed.node
}

fn recover_texxet_directions_list<G>(
    stores: &mut CommandContext<'_, G>,
    source: PageListId,
    expected: &mut Vec<tex_state::node::Direction>,
) -> (PageListId, Option<(usize, usize)>) {
    if stores.int_param(tex_state::env::banks::IntParam::TEX_XET_STATE) <= 0 {
        return (source, None);
    }
    expected.clear();
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    let mut extra = 0usize;
    for index in 0..source.len() {
        let direction = stores
            .page_node_list(source)
            .expect("direction source belongs to the live page arena")
            .nodes()
            .owned_node(index)
            .and_then(|node| match node {
                Node::Direction(direction) => Some(*direction),
                _ => None,
            });
        let replacement = direction.and_then(|direction| {
            let closes = match direction {
                tex_state::node::Direction::BeginM => Some(tex_state::node::Direction::EndM),
                tex_state::node::Direction::BeginL => Some(tex_state::node::Direction::EndL),
                tex_state::node::Direction::BeginR => Some(tex_state::node::Direction::EndR),
                tex_state::node::Direction::EndM
                | tex_state::node::Direction::EndL
                | tex_state::node::Direction::EndR => None,
            };
            if let Some(closes) = closes {
                expected.push(closes);
                None
            } else if expected.last() == Some(&direction) {
                let _ = expected.pop();
                None
            } else {
                extra += 1;
                Some(Node::Kern {
                    amount: tex_state::scaled::Scaled::from_raw(0),
                    kind: tex_state::node::KernKind::Explicit,
                })
            }
        });
        if let Some(replacement) = replacement {
            stores.push_page_active_list(&mut output, replacement);
        } else {
            stores.append_page_active_list_range(&mut output, source, index..index + 1);
        }
    }
    let missing = expected.len();
    for index in (0..expected.len()).rev() {
        stores.push_page_active_list(&mut output, Node::Direction(expected[index]));
    }
    (
        stores.finalize_page_active_list(&mut output),
        (missing != 0 || extra != 0).then_some((missing, extra)),
    )
}

fn project_short_diagnostic_discs_list<G>(
    stores: &mut CommandContext<'_, G>,
    physical: PageListId,
    semantic: PageListId,
) -> PageListId {
    let mut semantic_index = 0;
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    for physical_index in 0..physical.len() {
        let physical_disc = stores
            .page_node_list(physical)
            .expect("physical diagnostic list remains live")
            .nodes()
            .owned_node(physical_index)
            .and_then(|node| match node {
                Node::Disc {
                    kind,
                    pre,
                    post,
                    replace,
                    physical_replace_count,
                } => Some((*kind, *pre, *post, *replace, *physical_replace_count)),
                _ => None,
            });
        let Some((kind, physical_pre, physical_post, replace, physical_replace_count)) =
            physical_disc
        else {
            stores.append_page_active_list_range(
                &mut output,
                physical,
                physical_index..physical_index + 1,
            );
            continue;
        };
        let mut semantic_pre_post = None;
        while semantic_index < semantic.len() {
            let candidate = stores
                .page_node_list(semantic)
                .expect("semantic diagnostic list remains live")
                .nodes()
                .owned_node(semantic_index)
                .and_then(|node| match node {
                    Node::Disc { pre, post, .. } => Some((*pre, *post)),
                    _ => None,
                });
            semantic_index += 1;
            if candidate.is_some() {
                semantic_pre_post = candidate;
                break;
            }
        }
        let (pre, post) = semantic_pre_post.unwrap_or((physical_pre, physical_post));
        stores.push_page_active_list(
            &mut output,
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            },
        );
    }
    stores.finalize_page_active_list(&mut output)
}

fn physical_discretionary_projection<G>(
    stores: &mut CommandContext<'_, G>,
    children: tex_state::node_arena::PageListId,
) -> Option<tex_state::node_arena::PageListId> {
    let nodes = stores
        .page_node_list(children)
        .expect("packed box belongs to the live page arena")
        .nodes();
    let replacements = nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| match node {
            Node::Disc {
                replace,
                physical_replace_count: 1..,
                ..
            } => Some((index, *replace)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if replacements.is_empty() {
        return None;
    }
    let source_len = nodes.len();
    let _ = nodes;
    let mut slices = Vec::new();
    let mut pieces = Vec::with_capacity(replacements.len().saturating_mul(2) + 1);
    let mut start = 0;
    for (index, replace) in replacements {
        pieces.push(stores.slice_page_node_sequence(children, start..index + 1, &mut slices));
        pieces.push(replace);
        start = index + 1;
    }
    if start < source_len {
        pieces.push(stores.slice_page_node_sequence(children, start..source_len, &mut slices));
    }
    Some(stores.compose_page_node_sequences(&pieces))
}

pub(crate) fn first_box_node<G>(
    stores: &CommandContext<'_, G>,
    owner: Option<PageListId>,
) -> Option<Node> {
    stores
        .page_node_list(owner?)
        .ok()?
        .get(0)
        .map(|node| node.to_owned_with(|id| id))
        .filter(|node| matches!(node, Node::HList(_) | Node::VList(_)))
}
