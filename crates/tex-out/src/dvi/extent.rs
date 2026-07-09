use tex_arith::Scaled;

use crate::{BoxNode, PageNode};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PageExtent {
    pub(super) height_depth: i32,
    pub(super) width: i32,
}

pub(super) fn page_extent(node: &PageNode) -> PageExtent {
    match node {
        PageNode::HList(box_node) | PageNode::VList(box_node) => box_extent(box_node),
        PageNode::Rule {
            width,
            height,
            depth,
        } => PageExtent {
            height_depth: optional_raw(*height) + optional_raw(*depth),
            width: optional_raw(*width),
        },
        PageNode::MathOn(width) | PageNode::MathOff(width) => PageExtent {
            height_depth: 0,
            width: width.raw(),
        },
        PageNode::Char { .. }
        | PageNode::Lig { .. }
        | PageNode::Kern { .. }
        | PageNode::Glue { .. }
        | PageNode::Penalty(_)
        | PageNode::Disc { .. }
        | PageNode::Mark { .. }
        | PageNode::Insert { .. }
        | PageNode::WhatsitAnchor { .. }
        | PageNode::Adjust(_) => PageExtent::default(),
    }
}

fn box_extent(box_node: &BoxNode) -> PageExtent {
    PageExtent {
        height_depth: box_node.height.raw() + box_node.depth.raw(),
        width: box_node.width.raw(),
    }
}

fn optional_raw(value: Option<Scaled>) -> i32 {
    value.map_or(0, Scaled::raw)
}
