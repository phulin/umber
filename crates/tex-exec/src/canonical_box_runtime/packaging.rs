//! Source-free horizontal packing and box-register lookup.

use tex_state::ids::NodeListId;
use tex_state::node::Node;
use tex_state::{GeometryObservation, Universe};
use tex_typeset::{PackDiagnostic, PackSpec, plan_hpack_nodes};

use crate::packing_params::{hpack_params, recover_texxet_directions};

pub(crate) fn hpack_with_overfull_rule(
    stores: &mut Universe,
    children: NodeListId,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = hpack_params(stores);
    let (mut packed, lr_problems) =
        crate::packing_params::hpack_unreported(stores, children, spec, params);
    if !stores.nodes(packed.node.children).is_empty()
        && params.overfull_rule.raw() > 0
        && packed
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, PackDiagnostic::Overfull { .. }))
    {
        let mut nodes = stores.nodes(packed.node.children).to_vec();
        nodes.push(Node::Rule {
            width: Some(params.overfull_rule),
            height: None,
            depth: None,
        });
        packed.node.children = stores.freeze_node_list(&nodes);
    }
    crate::packing_params::report_hpack(stores, &packed, lr_problems);
    packed.node
}

pub(crate) fn hpack_owned_with_overfull_rule(
    stores: &mut Universe,
    nodes: &mut Vec<Node>,
    mut diagnostic_nodes: Option<&mut Vec<Node>>,
    spec: PackSpec,
) -> tex_state::node::BoxNode {
    let params = hpack_params(stores);
    let lr_problems = recover_texxet_directions(stores, nodes);
    if let Some(diagnostic_nodes) = diagnostic_nodes.as_deref_mut() {
        let _ = recover_texxet_directions(stores, diagnostic_nodes);
    }
    let plan = plan_hpack_nodes(stores, nodes, spec, params);
    if !nodes.is_empty()
        && params.overfull_rule.raw() > 0
        && plan
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, PackDiagnostic::Overfull { .. }))
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
    let children = stores.freeze_node_list_owned(nodes);
    let mut packed = plan.finish(children);
    stores.set_last_badness(packed.badness);
    stores.record_geometry_observation(GeometryObservation::Hpack {
        width_sp: i64::from(packed.node.width.raw()),
        height_sp: i64::from(packed.node.height.raw()),
        depth_sp: i64::from(packed.node.depth.raw()),
        line: stores.current_input_line().max(0) as u32,
        source: stores.current_input_source(),
    });
    let diagnostic_box = diagnostic_nodes.map_or(packed.node, |nodes| {
        let diagnostic_children = stores.freeze_node_list(nodes);
        let children = stores.freeze_node_list(
            short_diagnostic_nodes
                .as_deref()
                .expect("physical diagnostics have a short-display projection"),
        );
        packed.node.diagnostic_children = Some(diagnostic_children);
        tex_state::node::BoxNode {
            children,
            ..packed.node
        }
    });
    crate::pack_report::report_pack_diagnostics(
        stores,
        crate::pack_report::PackedDirection::Horizontal,
        &packed.diagnostics,
        &Node::HList(diagnostic_box),
        diagnostic_list_layout,
    );
    if let Some((missing, extra)) = lr_problems {
        crate::pack_report::report_lr_problems(
            stores,
            missing,
            extra,
            &Node::HList(diagnostic_box),
            diagnostic_list_layout,
        );
    }
    packed.node
}

pub(crate) fn project_short_diagnostic_discs(physical: &[Node], semantic: &[Node]) -> Vec<Node> {
    let mut semantic_discs = semantic.iter().filter_map(|node| match node {
        Node::Disc { pre, post, .. } => Some((*pre, *post)),
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
                let (pre, post) = semantic_discs.next().unwrap_or((*pre, *post));
                Node::Disc {
                    kind: *kind,
                    pre,
                    post,
                    replace: *replace,
                    physical_replace_count: *physical_replace_count,
                }
            }
            _ => node.clone(),
        })
        .collect()
}

pub(crate) fn first_box_node(stores: &Universe, id: Option<NodeListId>) -> Option<Node> {
    let id = id?;
    stores.nodes(id).first().and_then(|node| match node {
        tex_state::node_arena::NodeRef::HList(_) | tex_state::node_arena::NodeRef::VList(_) => {
            Some(node.to_owned())
        }
        _ => None,
    })
}
