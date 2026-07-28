use tex_state::Universe;
#[cfg(test)]
mod tests;

use tex_command::FatalError;
use tex_state::node::{Node, UnsetKind, UnsetNode, UnsetNodeFields};
use tex_typeset::measure_unset;

use crate::ExecError;
use crate::mode::AlignmentKind;

/// TeX82 §110's `max_quarterword`, the largest value §797's quarterword
/// `span_count` field can hold.
const MAX_QUARTERWORD: u16 = 255;

/// TeX82 §796's `type(u):=unset_node; span_count(u):=n`.
///
/// `span_count` is Umber's 1-based column count, so it is one more than
/// §796's `n`, which starts at `min_quarterword` because "this represents a
/// span count of 1" and is then incremented once per column step by §798's
/// `repeat incr(n); q:=link(link(q)); until q=cur_align`.
///
/// §798 guards that walk with
/// `if n>max_quarterword then confusion("256 spans")`, and this is the single
/// place any alignment row or column commits a span count to the quarterword
/// field, so the guard belongs here rather than at each individual caller --
/// every present and future §796 packaging site inherits it.
pub(crate) fn make_unset_node(
    stores: &Universe,
    children: tex_state::ids::NodeListId,
    kind: UnsetKind,
    span_count: u16,
) -> Result<Node, ExecError> {
    if span_count.saturating_sub(1) > MAX_QUARTERWORD {
        return Err(ExecError::Fatal(FatalError::confusion("256 spans")));
    }
    let metrics = measure_unset(stores, children, kind);
    Ok(Node::Unset(UnsetNode::new(UnsetNodeFields {
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
    })))
}

pub(crate) fn cell_unset_kind(kind: AlignmentKind) -> UnsetKind {
    match kind {
        AlignmentKind::HAlign => UnsetKind::HBox,
        AlignmentKind::VAlign => UnsetKind::VBox,
    }
}

pub(crate) fn row_unset_kind(kind: AlignmentKind) -> UnsetKind {
    cell_unset_kind(kind)
}
