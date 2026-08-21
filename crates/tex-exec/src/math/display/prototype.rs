use tex_state::Universe;
use tex_state::env::banks::GlueParam;
use tex_state::glue::GlueSpec;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_typeset::PackSpec;

use super::{scaled_add, scaled_sub};
use crate::packing_params::{hpack as hpack_nodes, hpack_params};

/// Merged e-TeX §§1475 and 1478's saved prototype for a display preceded by
/// a nonempty paragraph. Only the two skip boundaries and the finalized last
/// line's box setting survive until `app_display`; the paragraph material
/// itself remains owned by the vertical list.
pub(crate) fn display_line_prototype<G>(stores: &mut Universe<G>, last_line: BoxNode) -> BoxNode {
    let boundary = |stores: &Universe<G>, parameter, kind| {
        let spec = stores.glue_param(parameter);
        if stores.glue(spec) == GlueSpec::ZERO {
            Node::Kern {
                amount: Scaled::from_raw(0),
                kind: KernKind::Font,
            }
        } else {
            Node::Glue {
                spec: *stores.glue(spec),
                kind,
                leader: None,
            }
        }
    };
    let children = [
        boundary(stores, GlueParam::LEFT_SKIP, GlueKind::LeftSkip),
        boundary(stores, GlueParam::RIGHT_SKIP, GlueKind::RightSkip),
    ];
    let children = stores.publish_page_nodes(&children);
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
pub(super) fn package_directed_display_line<G>(
    stores: &mut Universe<G>,
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

    let mut payload = if display_line.box_lr == BoxLr::DList {
        vec![Node::HList(display_line.clone())]
    } else {
        let mut children = stores
            .page_node_list(display_line.children)
            .expect("display line belongs to the live page arena")
            .nodes()
            .to_vec();
        if pre_display_direction < 0 {
            children.reverse();
        }
        children
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
        let [left, right] = stores
            .page_node_list(prototype.children)
            .expect("display prototype belongs to the live page arena")
            .nodes()
            .to_vec()
            .try_into()
            .unwrap_or_else(|_| panic!("e-TeX display prototype has exactly two boundaries"));
        let mut children = Vec::with_capacity(payload.len() + 6);
        match left {
            Node::Glue {
                spec,
                kind,
                leader: None,
            } => {
                children.push(Node::Glue {
                    spec,
                    kind,
                    leader: None,
                });
                children.push(Node::Direction(tex_state::node::Direction::BeginM));
                children.push(cancel_display_skip(stores, &spec, kind, displacement));
            }
            Node::Kern { kind, .. } => {
                children.push(Node::Direction(tex_state::node::Direction::BeginM));
                children.push(Node::Kern {
                    amount: displacement,
                    kind,
                });
            }
            _ => panic!("e-TeX display prototype left boundary is glue or kern"),
        }
        children.append(&mut payload);
        match right {
            Node::Glue {
                spec,
                kind,
                leader: None,
            } => {
                children.push(cancel_display_skip(stores, &spec, kind, end_displacement));
                children.push(Node::Direction(tex_state::node::Direction::EndM));
                children.push(Node::Glue {
                    spec,
                    kind,
                    leader: None,
                });
            }
            Node::Kern { kind, .. } => {
                children.push(Node::Kern {
                    amount: end_displacement,
                    kind,
                });
                children.push(Node::Direction(tex_state::node::Direction::EndM));
            }
            _ => panic!("e-TeX display prototype right boundary is glue or kern"),
        }
        let children = stores.publish_page_nodes_owned(&mut children);
        prototype.children = children;
        return prototype;
    }

    let mut children = Vec::with_capacity(payload.len() + 4);
    children.push(Node::Direction(tex_state::node::Direction::BeginM));
    children.push(Node::Kern {
        amount: displacement,
        kind: KernKind::Font,
    });
    children.append(&mut payload);
    children.push(Node::Kern {
        amount: end_displacement,
        kind: KernKind::Font,
    });
    children.push(Node::Direction(tex_state::node::Direction::EndM));
    let list = stores.publish_page_nodes(&children);
    let mut boxed = hpack_nodes(stores, list, PackSpec::Natural, hpack_params(stores)).node;
    boxed.shift = display_indent;
    boxed
}

fn cancel_display_skip<G>(
    stores: &mut Universe<G>,
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
