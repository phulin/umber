use tex_state::CommandContext;
use tex_state::env::banks::GlueParam;
use tex_state::glue::GlueSpec;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_typeset::PackSpec;

use super::{scaled_add, scaled_sub};
use crate::packing_params::{hpack as hpack_nodes, hpack_params};

#[derive(Clone, Copy)]
enum PrototypeBoundary {
    Glue(GlueSpec, GlueKind),
    Kern(KernKind),
}

/// Merged e-TeX §§1475 and 1478's saved prototype for a display preceded by
/// a nonempty paragraph. Only the two skip boundaries and the finalized last
/// line's box setting survive until `app_display`; the paragraph material
/// itself remains owned by the vertical list.
pub(crate) fn display_line_prototype<G>(
    stores: &mut CommandContext<'_, G>,
    last_line: BoxNode,
) -> BoxNode {
    let boundary = |stores: &CommandContext<'_, G>, parameter, kind| {
        let spec = stores
            .glue_param(parameter)
            .map_or(GlueSpec::ZERO, |id| stores.glue(id));
        if spec == GlueSpec::ZERO {
            Node::Kern {
                amount: Scaled::from_raw(0),
                kind: KernKind::Font,
            }
        } else {
            Node::Glue {
                spec,
                kind,
                leader: None,
            }
        }
    };
    let children = [
        boundary(stores, GlueParam::LEFT_SKIP, GlueKind::LeftSkip),
        boundary(stores, GlueParam::RIGHT_SKIP, GlueKind::RightSkip),
    ];
    let children = stores.publish_page_nodes(children.into());
    BoxNode::new(BoxNodeFields {
        width: last_line.width,
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: last_line.shift,
        box_lr: BoxLr::Normal,
        glue_set: last_line.glue_set,
        glue_sign: last_line.glue_sign,
        glue_order: last_line.glue_order,
        children,
    })
}

/// e-TeX §§1478–1480's nonzero-`pre_display_direction` `app_display` path.
///
/// The inner `dlist` identity belongs to the formula box. The line appended
/// to the vertical list is a normal hbox whose math-direction boundaries make
/// the display transparent to the surrounding TeXXeT paragraph direction.
#[allow(clippy::too_many_arguments)] // Display packaging preserves the independent §1200 geometry inputs.
pub(super) fn package_directed_display_line<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    display_line: BoxNode,
    prototype: Option<BoxNode>,
    mut displacement: Scaled,
    mut display_indent: Scaled,
    display_width: Scaled,
    pre_display_direction: i32,
) -> BoxNode {
    let mut end_displacement = if pre_display_direction > 0 {
        scaled_sub(scaled_sub(display_width, displacement), display_line.width)
    } else {
        let end = displacement;
        displacement = scaled_sub(scaled_sub(display_width, end), display_line.width);
        end
    };

    let payload = if display_line.box_lr == BoxLr::DList {
        stores.publish_page_nodes(vec![Node::HList(display_line.clone())])
    } else {
        let children = stores
            .page_node_list(display_line.children)
            .expect("display line belongs to the live page arena")
            .nodes();
        let len = children.len();
        drop(children);
        if pre_display_direction >= 0 {
            display_line.children
        } else {
            let mut slices = Vec::new();
            let pieces = (0..len)
                .rev()
                .map(|index| {
                    stores.slice_page_node_sequence(
                        display_line.children,
                        index..index + 1,
                        &mut slices,
                    )
                })
                .collect::<Vec<_>>();
            stores.compose_page_node_sequences(&pieces)
        }
    };
    if let Some(mut prototype) = prototype {
        // e-TeX §1479 copies the prototype and adjusts the two edge
        // displacements against its retained shift and width. Section 1480
        // then replaces its list directly: unlike the no-prototype branch,
        // this path does not call `hpack` a second time.
        prototype.height = display_line.height;
        prototype.depth = display_line.depth;
        display_indent = scaled_sub(display_indent, prototype.shift);
        displacement = scaled_add(displacement, display_indent);
        end_displacement = scaled_add(
            end_displacement,
            scaled_sub(scaled_sub(prototype.width, display_width), display_indent),
        );
        let boundaries = stores
            .page_node_list(prototype.children)
            .expect("display prototype belongs to the live page arena")
            .nodes();
        assert_eq!(
            boundaries.len(),
            2,
            "e-TeX display prototype has exactly two boundaries"
        );
        let left = match boundaries
            .owned_node(0)
            .expect("display prototype has a left boundary")
        {
            Node::Glue {
                spec,
                kind,
                leader: None,
            } => PrototypeBoundary::Glue(*spec, *kind),
            Node::Kern { kind, .. } => PrototypeBoundary::Kern(*kind),
            _ => panic!("e-TeX display prototype left boundary is glue or kern"),
        };
        let right = match boundaries
            .owned_node(1)
            .expect("display prototype has a right boundary")
        {
            Node::Glue {
                spec,
                kind,
                leader: None,
            } => PrototypeBoundary::Glue(*spec, *kind),
            Node::Kern { kind, .. } => PrototypeBoundary::Kern(*kind),
            _ => panic!("e-TeX display prototype right boundary is glue or kern"),
        };
        drop(boundaries);

        let mut slices = Vec::new();
        let mut pieces = Vec::with_capacity(5);
        let mut prefix = Vec::with_capacity(2);
        if let PrototypeBoundary::Glue(spec, kind) = left {
            pieces.push(stores.slice_page_node_sequence(prototype.children, 0..1, &mut slices));
            prefix.push(Node::Direction(tex_state::node::Direction::BeginM));
            prefix.push(cancel_display_skip(stores, &spec, kind, displacement));
        } else if let PrototypeBoundary::Kern(kind) = left {
            prefix.push(Node::Direction(tex_state::node::Direction::BeginM));
            prefix.push(Node::Kern {
                amount: displacement,
                kind,
            });
        }
        pieces.push(stores.publish_page_nodes(prefix));
        pieces.push(payload);

        let mut suffix = Vec::with_capacity(2);
        if let PrototypeBoundary::Glue(spec, kind) = right {
            suffix.push(cancel_display_skip(stores, &spec, kind, end_displacement));
            suffix.push(Node::Direction(tex_state::node::Direction::EndM));
            pieces.push(stores.publish_page_nodes(suffix));
            pieces.push(stores.slice_page_node_sequence(prototype.children, 1..2, &mut slices));
        } else if let PrototypeBoundary::Kern(kind) = right {
            suffix.push(Node::Kern {
                amount: end_displacement,
                kind,
            });
            suffix.push(Node::Direction(tex_state::node::Direction::EndM));
            pieces.push(stores.publish_page_nodes(suffix));
        }
        prototype.children = stores.compose_page_node_sequences(&pieces);
        return prototype;
    }

    let prefix = stores.publish_page_nodes(vec![
        Node::Direction(tex_state::node::Direction::BeginM),
        Node::Kern {
            amount: displacement,
            kind: KernKind::Font,
        },
    ]);
    let suffix = stores.publish_page_nodes(vec![
        Node::Kern {
            amount: end_displacement,
            kind: KernKind::Font,
        },
        Node::Direction(tex_state::node::Direction::EndM),
    ]);
    let list = stores.compose_page_node_sequences(&[prefix, payload, suffix]);
    let mut boxed = hpack_nodes(
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
        list,
        PackSpec::Natural,
        hpack_params(stores),
    )
    .node;
    boxed.shift = display_indent;
    boxed
}

fn cancel_display_skip<G>(
    _stores: &mut CommandContext<'_, G>,
    original: &tex_state::glue::GlueSpec,
    kind: GlueKind,
    displacement: Scaled,
) -> Node {
    let original = *original;
    let spec = GlueSpec {
        width: scaled_sub(displacement, original.width),
        stretch: original
            .stretch
            .checked_neg()
            .expect("e-TeX display skip stretch negation is in range"),
        stretch_order: original.stretch_order,
        shrink: original
            .shrink
            .checked_neg()
            .expect("e-TeX display skip shrink negation is in range"),
        shrink_order: original.shrink_order,
    };
    Node::Glue {
        spec,
        kind,
        leader: None,
    }
}
