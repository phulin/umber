mod debug;
mod resolution;
mod set;

#[cfg(test)]
mod tests;

use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::glue::GlueSpec;
use tex_state::node::{
    BoxNode, BoxNodeFields, GlueKind, Node, Sign, UnsetKind, UnsetNode, UnsetNodeFields,
};
use tex_state::node_arena::PageListId;
use tex_state::page_node_arena::PageMaterialActiveListBuilder;
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_typeset::{HpackParams, PackSpec};

use crate::ExecError;
use crate::mode::{AlignState, AlignmentKind, AlignmentPackSpec};
use crate::packing_params::{hpack, hpack_params as read_hpack_params, vpack, vpack_params};

/// Runs TeX82 §800 `fin_align`'s setting pass over the alignment's own list.
///
/// `offset` is §800's `o`: `if nest[nest_ptr-1].mode_field=mmode then
/// o:=display_indent else o:=0`. It is the shift §807 gives every row and
/// §806 gives every running rule, so it must be decided once, from the mode
/// enclosing the alignment level, and applied to both.
pub(crate) fn finish_alignment<G>(
    state: &AlignState,
    rows: PageListId,
    offset: Scaled,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
) -> Result<PageListId, ExecError> {
    let resolved = resolution::resolve_widths(
        state,
        stores
            .page_node_list(rows)
            .expect("alignment rows belong to the live page arena")
            .nodes(),
        stores,
    )?;
    let empty = PageListId::empty();
    let prototype = pack_prototype(
        state,
        &resolved,
        &empty,
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
    );
    let finished = set::set_alignment_nodes(
        state.kind(),
        rows,
        &resolved,
        &prototype,
        empty,
        offset,
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
    )?;
    debug::debug_assert_no_unset_nodes(stores, finished);
    Ok(finished)
}

#[derive(Clone, Debug)]
struct ResolvedWidths {
    columns: Vec<Scaled>,
    tabskips: Vec<GlueSpec>,
}

#[derive(Clone, Debug)]
struct Prototype {
    box_node: BoxNode,
}

fn pack_prototype<G>(
    state: &AlignState,
    resolved: &ResolvedWidths,
    empty: &PageListId,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
) -> Prototype {
    let list = prototype_nodes(state.kind(), resolved, empty, stores);
    let spec = pack_spec(state.pack_spec());
    let box_node = match state.kind() {
        AlignmentKind::HAlign => {
            hpack(
                stores,
                diagnostic_effects,
                geometry,
                diagnostic_context,
                list,
                spec,
                hpack_params(stores),
            )
            .node
        }
        AlignmentKind::VAlign => {
            vpack(
                stores,
                diagnostic_effects,
                geometry,
                diagnostic_context,
                list,
                spec,
                vpack_params(stores),
            )
            .node
        }
    };
    Prototype { box_node }
}

fn prototype_nodes<G>(
    kind: AlignmentKind,
    resolved: &ResolvedWidths,
    empty: &PageListId,
    stores: &mut CommandContext<'_, G>,
) -> PageListId {
    let mut builder = PageMaterialActiveListBuilder::vacant();
    stores.open_page_active_list(&mut builder);
    stores.push_page_active_list(&mut builder, tabskip_node(resolved.tabskips[0]));
    for (column, width) in resolved.columns.iter().copied().enumerate() {
        stores.push_page_active_list(&mut builder, prototype_column(kind, width, *empty));
        stores.push_page_active_list(&mut builder, tabskip_node(resolved.tabskips[column + 1]));
    }
    stores.finalize_page_active_list(&mut builder)
}

fn hpack_params<G>(stores: &CommandContext<'_, G>) -> HpackParams {
    let mut params = read_hpack_params(stores);
    params.overfull_rule = Scaled::from_raw(0);
    params
}

fn pack_spec(spec: AlignmentPackSpec) -> PackSpec {
    match spec {
        AlignmentPackSpec::Natural => PackSpec::Natural,
        AlignmentPackSpec::Exactly(size) => PackSpec::Exactly(size),
        AlignmentPackSpec::Spread(extra) => PackSpec::Spread(extra),
    }
}

fn unset_axis_size<List>(
    kind: AlignmentKind,
    unset: &UnsetNode<List>,
) -> Result<Scaled, ExecError> {
    match kind {
        AlignmentKind::HAlign => Ok(unset.width),
        AlignmentKind::VAlign => add_scaled(unset.height, unset.depth),
    }
}

/// TeX82 §805 changes every preamble column record to `unset_node` before
/// packing the prototype.  The subtype is observable when §663 reports a
/// loose prototype: `show_box` must therefore see the typed unset records,
/// even though ordinary packing measures them like boxes.
fn prototype_column(kind: AlignmentKind, size: Scaled, empty: PageListId) -> Node {
    let (unset_kind, width, height) = match kind {
        AlignmentKind::HAlign => (UnsetKind::HBox, size, Scaled::from_raw(0)),
        AlignmentKind::VAlign => (UnsetKind::VBox, Scaled::from_raw(0), size),
    };
    Node::Unset(UnsetNode::new(UnsetNodeFields {
        kind: unset_kind,
        width,
        height,
        depth: Scaled::from_raw(0),
        span_count: 0,
        stretch: Scaled::from_raw(0),
        stretch_order: tex_state::glue::Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: tex_state::glue::Order::Normal,
        children: empty,
    }))
}

fn empty_column_box(kind: AlignmentKind, size: Scaled, empty: PageListId) -> Node {
    let fields = match kind {
        AlignmentKind::HAlign => BoxNodeFields {
            width: size,
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: tex_state::glue::Order::Normal,
            children: empty,
        },
        AlignmentKind::VAlign => BoxNodeFields {
            width: Scaled::from_raw(0),
            height: size,
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: tex_state::glue::Order::Normal,
            children: empty,
        },
    };
    match kind {
        AlignmentKind::HAlign => Node::HList(BoxNode::new(fields)),
        AlignmentKind::VAlign => Node::VList(BoxNode::new(fields)),
    }
}

fn tabskip_node(spec: GlueSpec) -> Node {
    Node::Glue {
        spec,
        kind: GlueKind::TabSkip,
        leader: None,
    }
}

fn rounded_glue(ratio: GlueSetRatio, amount: Scaled) -> Result<Scaled, ExecError> {
    let product = i128::from(ratio.numerator()) * i128::from(amount.raw());
    let rounded = rounded_div(product, i128::from(ratio.denominator()));
    let raw = i32::try_from(rounded).map_err(|_| ExecError::ArithmeticOverflow)?;
    Ok(Scaled::from_raw(raw))
}

fn rounded_div(value: i128, divisor: i128) -> i128 {
    debug_assert!(divisor > 0);
    if value >= 0 {
        (value + divisor / 2) / divisor
    } else {
        -((-value + divisor / 2) / divisor)
    }
}

fn add_scaled(left: Scaled, right: Scaled) -> Result<Scaled, ExecError> {
    left.checked_add(right).ok_or(ExecError::ArithmeticOverflow)
}

fn scaled_from_i64(value: i64) -> Result<Scaled, ExecError> {
    let raw = i32::try_from(value).map_err(|_| ExecError::ArithmeticOverflow)?;
    Ok(Scaled::from_raw(raw))
}
