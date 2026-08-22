use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
#[cfg(test)]
mod tests;

use tex_command::FatalError;
use tex_state::node::{Node, UnsetKind, UnsetNode, UnsetNodeFields};
use tex_typeset::PackSpec;
use tex_typeset::measure_unset;

use crate::ExecError;
use crate::mode::AlignmentKind;
use crate::packing_params::{hpack, hpack_params, vpack, vpack_params};

/// TeX82 §110's `max_quarterword`, the largest value §797's quarterword
/// `span_count` field can hold.
const MAX_QUARTERWORD: u16 = 255;

#[derive(Clone, Copy)]
pub(crate) enum UnsetPackContext {
    Cell,
    Row,
}

/// TeX82 §796's `type(u):=unset_node; span_count(u):=n`.
///
/// The argument is Umber's 1-based column count. The stored field is §796's
/// encoded `n`, which starts at `min_quarterword` because "this represents a
/// span count of 1" and is then incremented once per column step by §798's
/// `repeat incr(n); q:=link(link(q)); until q=cur_align`.
///
/// §798 guards that walk with
/// `if n>max_quarterword then confusion("256 spans")`, and this is the single
/// place any alignment row or column commits a span count to the quarterword
/// field, so the guard belongs here rather than at each individual caller --
/// every present and future §796 packaging site inherits it.
#[allow(clippy::too_many_arguments)] // TeX unset packing keeps pack services and node facts explicit.
pub(crate) fn make_unset_node<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
    children: tex_state::node_arena::PageListId,
    kind: UnsetKind,
    span_count: u16,
    context: UnsetPackContext,
) -> Result<Node, ExecError> {
    if span_count.saturating_sub(1) > MAX_QUARTERWORD {
        return Err(ExecError::Fatal(FatalError::confusion("256 spans")));
    }
    // TeX82 §796 calls hpack/vpackage before changing the resulting box's
    // type to unset_node, and §799 likewise packs the completed row before
    // making it unset. Besides determining the retained dimensions, those
    // calls set \badness and are canonical packing transitions in their own
    // right; measuring the children directly silently skipped both effects.
    let packed = match kind {
        UnsetKind::HBox => hpack(
            stores,
            diagnostic_effects,
            geometry,
            diagnostic_context,
            children,
            PackSpec::Natural,
            hpack_params(stores),
        ),
        UnsetKind::VBox => {
            let mut params = vpack_params(stores);
            // TeX82 §796 uses `vpackage(link(head),natural,0)` for a valign
            // entry, whereas §799's completed valign row uses the `vpack`
            // macro and therefore `max_dimen`. The distinction is observable
            // whenever the entry has nonzero depth.
            params.box_max_depth = match context {
                UnsetPackContext::Cell => tex_state::scaled::Scaled::from_raw(0),
                UnsetPackContext::Row => tex_state::scaled::Scaled::MAX_DIMEN,
            };
            vpack(
                stores,
                diagnostic_effects,
                geometry,
                diagnostic_context,
                children,
                PackSpec::Natural,
                params,
            )
        }
    };
    let metrics = measure_unset(
        &crate::typeset_context::TypesetContext::new(stores),
        &packed.node.children,
        kind,
    );
    let children = packed.node.children;
    Ok(Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind,
        width: packed.node.width,
        height: packed.node.height,
        depth: packed.node.depth,
        span_count: span_count.saturating_sub(1),
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
