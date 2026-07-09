use tex_state::Universe;
use tex_state::node::{Node, UnsetKind, UnsetNode, UnsetNodeFields};
use tex_typeset::measure_unset;

use crate::mode::AlignmentKind;

pub(super) fn make_unset_node(
    stores: &Universe,
    children: tex_state::ids::NodeListId,
    kind: UnsetKind,
    span_count: u16,
) -> Node {
    let metrics = measure_unset(stores, children, kind);
    Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind,
        width: metrics.width,
        height: metrics.height,
        depth: metrics.depth,
        span_count,
        stretch: metrics.stretch,
        stretch_order: metrics.stretch_order,
        shrink: metrics.shrink,
        shrink_order: metrics.shrink_order,
        children,
    }))
}

pub(super) fn cell_unset_kind(kind: AlignmentKind) -> UnsetKind {
    match kind {
        AlignmentKind::HAlign => UnsetKind::HBox,
        AlignmentKind::VAlign => UnsetKind::VBox,
    }
}

pub(super) fn row_unset_kind(kind: AlignmentKind) -> UnsetKind {
    cell_unset_kind(kind)
}
