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
    let mut children = tex_state::page_node_arena::PageMaterialActiveListBuilder::vacant();
    stores.open_page_active_list(&mut children);
    stores.push_page_active_list(
        &mut children,
        boundary(stores, GlueParam::LEFT_SKIP, GlueKind::LeftSkip),
    );
    stores.push_page_active_list(
        &mut children,
        boundary(stores, GlueParam::RIGHT_SKIP, GlueKind::RightSkip),
    );
    let children = stores.finalize_page_active_list(&mut children);
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
        let mut payload = tex_state::page_node_arena::PageMaterialActiveListBuilder::vacant();
        stores.open_page_active_list(&mut payload);
        stores.push_page_active_list(&mut payload, Node::HList(display_line));
        stores.finalize_page_active_list(&mut payload)
    } else {
        let children = stores
            .page_node_list(display_line.children)
            .expect("display line belongs to the live page arena")
            .nodes();
        let len = children.len();
        if pre_display_direction >= 0 {
            display_line.children
        } else {
            let mut reversed = tex_state::page_node_arena::PageMaterialActiveListBuilder::vacant();
            stores.open_page_active_list(&mut reversed);
            for index in (0..len).rev() {
                stores.append_page_active_list_range(
                    &mut reversed,
                    display_line.children,
                    index..index + 1,
                );
            }
            stores.finalize_page_active_list(&mut reversed)
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

        let mut replacement = tex_state::page_node_arena::PageMaterialActiveListBuilder::vacant();
        stores.open_page_active_list(&mut replacement);
        if let PrototypeBoundary::Glue(spec, kind) = left {
            stores.append_page_active_list_range(&mut replacement, prototype.children, 0..1);
            stores.push_page_active_list(
                &mut replacement,
                Node::Direction(tex_state::node::Direction::BeginM),
            );
            let cancelled = cancel_display_skip(stores, &spec, kind, displacement);
            stores.push_page_active_list(&mut replacement, cancelled);
        } else if let PrototypeBoundary::Kern(kind) = left {
            stores.push_page_active_list(
                &mut replacement,
                Node::Direction(tex_state::node::Direction::BeginM),
            );
            stores.push_page_active_list(
                &mut replacement,
                Node::Kern {
                    amount: displacement,
                    kind,
                },
            );
        }
        stores.append_page_active_list(&mut replacement, payload);

        if let PrototypeBoundary::Glue(spec, kind) = right {
            let cancelled = cancel_display_skip(stores, &spec, kind, end_displacement);
            stores.push_page_active_list(&mut replacement, cancelled);
            stores.push_page_active_list(
                &mut replacement,
                Node::Direction(tex_state::node::Direction::EndM),
            );
            stores.append_page_active_list_range(&mut replacement, prototype.children, 1..2);
        } else if let PrototypeBoundary::Kern(kind) = right {
            stores.push_page_active_list(
                &mut replacement,
                Node::Kern {
                    amount: end_displacement,
                    kind,
                },
            );
            stores.push_page_active_list(
                &mut replacement,
                Node::Direction(tex_state::node::Direction::EndM),
            );
        }
        prototype.children = stores.finalize_page_active_list(&mut replacement);
        return prototype;
    }

    let mut list = tex_state::page_node_arena::PageMaterialActiveListBuilder::vacant();
    stores.open_page_active_list(&mut list);
    stores.push_page_active_list(
        &mut list,
        Node::Direction(tex_state::node::Direction::BeginM),
    );
    stores.push_page_active_list(
        &mut list,
        Node::Kern {
            amount: displacement,
            kind: KernKind::Font,
        },
    );
    stores.append_page_active_list(&mut list, payload);
    stores.push_page_active_list(
        &mut list,
        Node::Kern {
            amount: end_displacement,
            kind: KernKind::Font,
        },
    );
    stores.push_page_active_list(&mut list, Node::Direction(tex_state::node::Direction::EndM));
    let list = stores.finalize_page_active_list(&mut list);
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
