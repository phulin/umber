mod debug;
mod resolution;
mod set;

use tex_state::Universe;
use tex_state::ids::{GlueId, NodeListId};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node, Sign, UnsetNode};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_typeset::{HpackParams, PackSpec};

use crate::ExecError;
use crate::mode::{AlignState, AlignmentKind, AlignmentPackSpec};
use crate::packing_params::{hpack, hpack_params as read_hpack_params, vpack, vpack_params};

pub(super) fn finish_alignment(
    state: &AlignState,
    rows: &[Node],
    stores: &mut Universe,
) -> Result<Vec<Node>, ExecError> {
    let resolved = resolution::resolve_widths(state, rows, stores)?;
    let empty = stores.freeze_node_list(&[]);
    let prototype = pack_prototype(state, &resolved, empty, stores);
    let finished =
        set::set_alignment_nodes(state.kind(), rows, &resolved, &prototype, empty, stores)?;
    debug::debug_assert_no_unset_nodes(stores, &finished);
    Ok(finished)
}

#[derive(Clone, Debug)]
struct ResolvedWidths {
    columns: Vec<Scaled>,
    tabskips: Vec<GlueId>,
}

#[derive(Clone, Debug)]
struct Prototype {
    box_node: BoxNode,
}

fn pack_prototype(
    state: &AlignState,
    resolved: &ResolvedWidths,
    empty: NodeListId,
    stores: &mut Universe,
) -> Prototype {
    let nodes = prototype_nodes(state.kind(), resolved, empty);
    let list = stores.freeze_node_list(&nodes);
    let spec = pack_spec(state.pack_spec());
    let box_node = match state.kind() {
        AlignmentKind::HAlign => hpack(stores, list, spec, hpack_params(stores)).node,
        AlignmentKind::VAlign => vpack(stores, list, spec, vpack_params(stores)).node,
    };
    Prototype { box_node }
}

fn prototype_nodes(kind: AlignmentKind, resolved: &ResolvedWidths, empty: NodeListId) -> Vec<Node> {
    let mut nodes = Vec::with_capacity(resolved.columns.len().saturating_mul(2) + 1);
    nodes.push(tabskip_node(resolved.tabskips[0]));
    for (column, width) in resolved.columns.iter().copied().enumerate() {
        nodes.push(empty_column_box(kind, width, empty));
        nodes.push(tabskip_node(resolved.tabskips[column + 1]));
    }
    nodes
}

fn hpack_params(stores: &Universe) -> HpackParams {
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

fn unset_axis_size(kind: AlignmentKind, unset: &UnsetNode) -> Scaled {
    match kind {
        AlignmentKind::HAlign => unset.width,
        AlignmentKind::VAlign => unset.height,
    }
}

fn empty_column_box(kind: AlignmentKind, size: Scaled, empty: NodeListId) -> Node {
    let fields = match kind {
        AlignmentKind::HAlign => BoxNodeFields {
            width: size,
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            display: false,
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
            display: false,
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

fn tabskip_node(spec: GlueId) -> Node {
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

fn sub_scaled(left: Scaled, right: Scaled) -> Result<Scaled, ExecError> {
    left.checked_sub(right).ok_or(ExecError::ArithmeticOverflow)
}

fn scaled_from_i64(value: i64) -> Result<Scaled, ExecError> {
    let raw = i32::try_from(value).map_err(|_| ExecError::ArithmeticOverflow)?;
    Ok(Scaled::from_raw(raw))
}
