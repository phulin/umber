//! Source-free horizontal packing and box-register lookup.

use tex_state::CommandContext;
use tex_state::node::Node;
use tex_state::node_arena::PageListId;
use tex_typeset::{PackDiagnostic, PackSpec, plan_hpack_nodes};

use crate::packing_params::{hpack_params, recover_texxet_directions};
use crate::{ExecError, Mode, ModeNest};

use super::hmode::flush_pending_hchars;

pub(crate) fn take_last_box<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    fuel: &mut tex_command::CommandFuel,
    error_context: String,
) -> Result<Option<Node>, ExecError> {
    flush_pending_hchars(nest, stores, fuel)?;
    match nest.current_mode() {
        Mode::Math | Mode::DisplayMath => {
            report_cannot_take_last_box(
                stores,
                "math mode",
                &["Sorry; this \\lastbox will be void."],
                error_context,
            )?;
            Ok(None)
        }
        Mode::Vertical
            if nest.current_list().is_empty() && stores.page_contributions().is_empty() =>
        {
            report_cannot_take_last_box(
                stores,
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
            let mut removed = stores.remove_page_contribution_range(tail.removal_range());
            Ok(reset_removed_box_shift(&mut removed))
        }
        Mode::InternalVertical | Mode::Horizontal | Mode::RestrictedHorizontal => {
            let Some(tail) =
                crate::effective_tail::EffectiveTail::find(nest.current_list().nodes().iter())
            else {
                return Ok(None);
            };
            if !matches!(tail.node(), Node::HList(_) | Node::VList(_)) {
                return Ok(None);
            }
            let range = tail.removal_range();
            let mut removed = nest.current_list_mutation().remove_node_range(range);
            Ok(reset_removed_box_shift(&mut removed))
        }
    }
}

fn reset_removed_box_shift(removed: &mut [Node]) -> Option<Node> {
    let node = removed
        .iter_mut()
        .find(|node| matches!(node, Node::HList(_) | Node::VList(_)))?;
    match node {
        Node::HList(box_node) | Node::VList(box_node) => {
            box_node.shift = tex_state::scaled::Scaled::from_raw(0);
        }
        _ => unreachable!("node was selected as a box"),
    }
    Some(node.clone())
}

fn report_cannot_take_last_box<G>(
    stores: &mut CommandContext<'_, G>,
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
    report.error().jump_out()?;
    Ok(())
}

pub(crate) fn hpack_with_overfull_rule<G>(
    stores: &mut CommandContext<'_, G>,
    context: &crate::pack_report::ExecutionDiagnosticContext,
    children: tex_state::node_arena::PageListId,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = hpack_params(stores);
    let (mut packed, lr_problems) =
        crate::packing_params::hpack_unreported(stores, children, spec, params);
    if !packed.node.children.is_empty()
        && params.overfull_rule.raw() > 0
        && packed
            .diagnostics
            .iter()
            .any(|diagnostic| {
                matches!(diagnostic, PackDiagnostic::Overfull { excess } if *excess > params.hfuzz)
            })
    {
        let mut nodes = stores
            .page_node_list(packed.node.children)
            .expect("packed box belongs to the live page arena")
            .nodes()
            .to_vec();
        nodes.push(Node::Rule {
            width: Some(params.overfull_rule),
            height: None,
            depth: None,
        });
        let children = stores.publish_page_nodes(nodes);
        packed.node.children = children;
    }
    // TeX82 §§115/162 stores a discretionary's replacement as the
    // physical nodes immediately following the disc node. Umber keeps that
    // material in `replace` so semantic list traversal cannot count it twice;
    // retain the physical projection separately for §182's diagnostic walk.
    if let Some(diagnostic_children) =
        physical_discretionary_projection(stores, packed.node.children.clone())
    {
        packed.node.diagnostic_children = Some(diagnostic_children);
    }
    crate::packing_params::report_hpack(stores, context, &packed, lr_problems);
    packed.node
}

fn physical_discretionary_projection<G>(
    stores: &mut CommandContext<'_, G>,
    children: tex_state::node_arena::PageListId,
) -> Option<tex_state::node_arena::PageListId> {
    let nodes = stores
        .page_node_list(children)
        .expect("packed box belongs to the live page arena")
        .nodes()
        .to_vec();
    if !nodes.iter().any(|node| {
        matches!(
            node,
            Node::Disc {
                physical_replace_count: 1..,
                ..
            }
        )
    }) {
        return None;
    }
    let mut physical = Vec::with_capacity(nodes.len());
    for node in nodes {
        let replace = match &node {
            Node::Disc { replace, .. } => Some(
                stores
                    .page_node_list(*replace)
                    .expect("discretionary replacement belongs to the live page arena")
                    .nodes()
                    .to_vec(),
            ),
            _ => None,
        };
        physical.push(node);
        if let Some(replace) = replace {
            physical.extend(replace);
        }
    }
    let physical = stores.publish_page_nodes(physical);
    Some(physical)
}

pub(crate) fn hpack_owned_with_overfull_rule<G>(
    stores: &mut CommandContext<'_, G>,
    context: &crate::pack_report::ExecutionDiagnosticContext,
    nodes: &mut Vec<Node>,
    mut diagnostic_nodes: Option<&mut Vec<Node>>,
    allocator_high_cell_overlap: u32,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = hpack_params(stores);
    let lr_problems = recover_texxet_directions(stores, nodes);
    if let Some(diagnostic_nodes) = diagnostic_nodes.as_deref_mut() {
        let _ = recover_texxet_directions(stores, diagnostic_nodes);
    }
    let plan = plan_hpack_nodes(
        &crate::typeset_context::TypesetContext::new(stores),
        nodes,
        spec,
        params,
    );
    if !nodes.is_empty()
        && params.overfull_rule.raw() > 0
        && plan
            .diagnostics
            .iter()
            .any(|diagnostic| {
                matches!(diagnostic, PackDiagnostic::Overfull { excess } if *excess > params.hfuzz)
            })
    {
        nodes.push(Node::Rule {
            width: Some(params.overfull_rule),
            height: None,
            depth: None,
        });
        if let Some(diagnostic_nodes) = diagnostic_nodes.as_deref_mut() {
            diagnostic_nodes.push(Node::Rule {
                width: Some(params.overfull_rule),
                height: None,
                depth: None,
            });
        }
    }
    let short_diagnostic_nodes = diagnostic_nodes
        .as_deref()
        .map(|physical| project_short_diagnostic_discs(physical, nodes));
    let diagnostic_list_layout = if short_diagnostic_nodes.is_some() {
        crate::pack_report::DiagnosticListLayout::DetachedProjection
    } else {
        crate::pack_report::DiagnosticListLayout::FrozenList
    };
    let children = stores.publish_page_nodes(std::mem::take(nodes));
    let mut packed = plan.finish(children);
    packed.node.allocator_high_cell_overlap = if diagnostic_nodes.is_some() {
        allocator_high_cell_overlap
    } else {
        0
    };
    stores.set_last_badness(packed.badness);
    let diagnostic_box = if let Some(nodes) = diagnostic_nodes {
        let diagnostic_children = stores.publish_page_nodes(std::mem::take(nodes));
        let children = stores.publish_page_nodes(
            short_diagnostic_nodes
                .as_deref()
                .expect("physical diagnostics have a short-display projection")
                .to_vec(),
        );
        packed.node.diagnostic_children = Some(diagnostic_children);
        tex_state::node::BoxNode {
            children,
            ..packed.node.clone()
        }
    } else {
        packed.node.clone()
    };
    crate::pack_report::report_pack_diagnostics(
        stores,
        context,
        crate::pack_report::PackedDirection::Horizontal,
        &packed.diagnostics,
        &Node::HList(diagnostic_box.clone()),
        diagnostic_list_layout,
    );
    if let Some((missing, extra)) = lr_problems {
        crate::pack_report::report_lr_problems(
            stores,
            context,
            missing,
            extra,
            &Node::HList(diagnostic_box.clone()),
            diagnostic_list_layout,
        );
    }
    packed.node
}

pub(crate) fn project_short_diagnostic_discs(physical: &[Node], semantic: &[Node]) -> Vec<Node> {
    let mut semantic_discs = semantic.iter().filter_map(|node| match node {
        Node::Disc { pre, post, .. } => Some((pre.clone(), post.clone())),
        _ => None,
    });
    physical
        .iter()
        .map(|node| match node {
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => {
                let (pre, post) = semantic_discs
                    .next()
                    .unwrap_or_else(|| (pre.clone(), post.clone()));
                Node::Disc {
                    kind: *kind,
                    pre,
                    post,
                    replace: replace.clone(),
                    physical_replace_count: *physical_replace_count,
                }
            }
            _ => node.clone(),
        })
        .collect()
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
