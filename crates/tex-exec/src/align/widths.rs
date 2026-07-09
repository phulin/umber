use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam};
use tex_state::glue::GlueSpec;
use tex_state::glue::Order;
use tex_state::ids::{GlueId, NodeListId};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node, Sign, UnsetNode};
use tex_state::scaled::{GLUE_SET_RATIO_SCALE, GlueSetRatio, Scaled};
use tex_typeset::{HpackParams, PackSpec, VpackParams, hpack, vpack};

use crate::ExecError;
use crate::mode::{AlignState, AlignmentKind, AlignmentPackSpec};

pub(super) fn finish_alignment(
    state: &AlignState,
    rows: &[Node],
    stores: &mut Universe,
) -> Result<Vec<Node>, ExecError> {
    let resolved = resolve_widths(state, rows, stores)?;
    let empty = stores.freeze_node_list(&[]);
    let prototype = pack_prototype(state, &resolved, empty, stores);
    let finished = set_alignment_nodes(state.kind(), rows, &resolved, &prototype, empty, stores)?;
    debug_assert_no_unset_nodes(stores, &finished);
    Ok(finished)
}

#[derive(Clone, Debug, Default)]
struct ColumnRecord {
    width: Option<Scaled>,
    spans: Vec<SpanRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SpanRecord {
    count: usize,
    width: Scaled,
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

#[derive(Clone, Copy)]
struct SetConfig<'a> {
    kind: AlignmentKind,
    resolved: &'a ResolvedWidths,
    prototype: &'a Prototype,
    empty: NodeListId,
}

fn resolve_widths(
    state: &AlignState,
    rows: &[Node],
    stores: &Universe,
) -> Result<ResolvedWidths, ExecError> {
    let mut records = vec![ColumnRecord::default(); state.columns().len()];
    let mut tabskips = initial_tabskips(state, records.len());
    collect_width_requirements(
        state.kind(),
        rows,
        stores,
        state,
        &mut records,
        &mut tabskips,
    );

    let mut index = 0;
    while index < records.len() {
        if records[index].width.is_none() {
            records[index].width = Some(Scaled::from_raw(0));
            ensure_layout_len(&mut records, &mut tabskips, state, index + 1);
            tabskips[index + 1] = GlueId::ZERO;
        }

        let spans = std::mem::take(&mut records[index].spans);
        if !spans.is_empty() {
            ensure_layout_len(&mut records, &mut tabskips, state, index + 2);
            let reduction = add_scaled(
                records[index].width.expect("column width was resolved"),
                stores.glue(tabskips[index + 1]).width,
            )?;
            for span in spans {
                let reduced = sub_scaled(span.width, reduction)?;
                merge_width(&mut records[index + 1], span.count - 1, reduced);
            }
        }
        index += 1;
    }

    Ok(ResolvedWidths {
        columns: records
            .into_iter()
            .map(|record| record.width.expect("all columns are resolved"))
            .collect(),
        tabskips,
    })
}

fn initial_tabskips(state: &AlignState, columns: usize) -> Vec<GlueId> {
    (0..=columns)
        .map(|boundary| state.tabskip_for_boundary(boundary))
        .collect()
}

fn collect_width_requirements(
    kind: AlignmentKind,
    rows: &[Node],
    stores: &Universe,
    state: &AlignState,
    records: &mut Vec<ColumnRecord>,
    tabskips: &mut Vec<GlueId>,
) {
    for node in rows {
        let Node::Unset(row) = node else {
            continue;
        };
        let mut column = 0usize;
        for child in stores.nodes(row.children) {
            let Node::Unset(cell) = child else {
                continue;
            };
            let span = usize::from(cell.span_count.max(1));
            ensure_layout_len(records, tabskips, state, column + span);
            merge_width(&mut records[column], span, unset_axis_size(kind, cell));
            column += span;
        }
    }
}

fn ensure_layout_len(
    records: &mut Vec<ColumnRecord>,
    tabskips: &mut Vec<GlueId>,
    state: &AlignState,
    columns: usize,
) {
    while records.len() < columns {
        records.push(ColumnRecord::default());
    }
    while tabskips.len() <= records.len() {
        let boundary = tabskips.len();
        tabskips.push(state.tabskip_for_boundary(boundary));
    }
}

fn merge_width(record: &mut ColumnRecord, count: usize, width: Scaled) {
    if count <= 1 {
        if record.width.is_none_or(|old| width > old) {
            record.width = Some(width);
        }
        return;
    }

    match record.spans.binary_search_by_key(&count, |span| span.count) {
        Ok(index) => {
            if width > record.spans[index].width {
                record.spans[index].width = width;
            }
        }
        Err(index) => record.spans.insert(index, SpanRecord { count, width }),
    }
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
        AlignmentKind::VAlign => vpack(stores, list, spec, VpackParams::read(stores)).node,
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
    let mut params = HpackParams::read(stores);
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

fn set_alignment_nodes(
    kind: AlignmentKind,
    rows: &[Node],
    resolved: &ResolvedWidths,
    prototype: &Prototype,
    empty: NodeListId,
    stores: &mut Universe,
) -> Result<Vec<Node>, ExecError> {
    let config = SetConfig {
        kind,
        resolved,
        prototype,
        empty,
    };
    let mut out = Vec::with_capacity(rows.len());
    let mut previous_row_depth = None;
    for node in rows {
        match node {
            Node::Unset(row) => {
                if config.kind == AlignmentKind::HAlign
                    && let Some(previous_depth) = previous_row_depth
                {
                    out.push(baseline_glue(previous_depth, row.height, stores)?);
                }
                let set = set_row(config, row, stores)?;
                previous_row_depth = if config.kind == AlignmentKind::HAlign {
                    row_depth(&set)
                } else {
                    None
                };
                out.push(set);
            }
            _ => {
                previous_row_depth = None;
                out.push(set_noalign_node(
                    config.kind,
                    node,
                    &config.prototype.box_node,
                ));
            }
        }
    }
    Ok(out)
}

fn baseline_glue(
    previous_depth: Scaled,
    height: Scaled,
    stores: &mut Universe,
) -> Result<Node, ExecError> {
    let baseline = stores.glue(stores.glue_param(GlueParam::BASELINE_SKIP));
    let requested = baseline
        .width
        .checked_sub(previous_depth)
        .and_then(|value| value.checked_sub(height))
        .ok_or(ExecError::ArithmeticOverflow)?;
    let (spec, kind) = if requested.raw() < stores.dimen_param(DimenParam::LINE_SKIP_LIMIT).raw() {
        (stores.glue_param(GlueParam::LINE_SKIP), GlueKind::LineSkip)
    } else {
        (
            stores.intern_glue(GlueSpec {
                width: requested,
                stretch: baseline.stretch,
                stretch_order: baseline.stretch_order,
                shrink: baseline.shrink,
                shrink_order: baseline.shrink_order,
            }),
            GlueKind::BaselineSkip,
        )
    };
    Ok(Node::Glue { spec, kind })
}

fn row_depth(node: &Node) -> Option<Scaled> {
    match node {
        Node::HList(box_node) | Node::VList(box_node) => Some(box_node.depth),
        _ => None,
    }
}

fn set_noalign_node(kind: AlignmentKind, node: &Node, prototype: &BoxNode) -> Node {
    match (kind, node) {
        (
            AlignmentKind::HAlign,
            Node::Rule {
                width: None,
                height,
                depth,
            },
        ) => Node::Rule {
            width: Some(prototype.width),
            height: *height,
            depth: *depth,
        },
        (
            AlignmentKind::VAlign,
            Node::Rule {
                width,
                height: None,
                depth,
            },
        ) => Node::Rule {
            width: *width,
            height: Some(prototype.height),
            depth: *depth,
        },
        _ => node.clone(),
    }
}

fn set_row(
    config: SetConfig<'_>,
    row: &UnsetNode,
    stores: &mut Universe,
) -> Result<Node, ExecError> {
    let children = set_row_children(config, row, stores)?;
    let children = stores.freeze_node_list(&children);
    let fields = match config.kind {
        AlignmentKind::HAlign => BoxNodeFields {
            width: config.prototype.box_node.width,
            height: row.height,
            depth: row.depth,
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: config.prototype.box_node.glue_set,
            glue_sign: config.prototype.box_node.glue_sign,
            glue_order: config.prototype.box_node.glue_order,
            children,
        },
        AlignmentKind::VAlign => BoxNodeFields {
            width: row.width,
            height: config.prototype.box_node.height,
            depth: row.depth,
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: config.prototype.box_node.glue_set,
            glue_sign: config.prototype.box_node.glue_sign,
            glue_order: config.prototype.box_node.glue_order,
            children,
        },
    };
    Ok(match config.kind {
        AlignmentKind::HAlign => Node::HList(BoxNode::new(fields)),
        AlignmentKind::VAlign => Node::VList(BoxNode::new(fields)),
    })
}

fn set_row_children(
    config: SetConfig<'_>,
    row: &UnsetNode,
    stores: &Universe,
) -> Result<Vec<Node>, ExecError> {
    let mut out = Vec::new();
    let mut column = 0usize;
    for child in stores.nodes(row.children) {
        match child {
            Node::Unset(cell) => {
                let span = usize::from(cell.span_count.max(1));
                out.push(set_cell(config, row, cell, column, span, stores)?);
                for offset in 1..span {
                    let spanned_column = column + offset;
                    out.push(tabskip_node(config.resolved.tabskips[spanned_column]));
                    out.push(empty_column_box(
                        config.kind,
                        config.resolved.columns[spanned_column],
                        config.empty,
                    ));
                }
                column += span;
            }
            _ => out.push(child.clone()),
        }
    }
    Ok(out)
}

fn set_cell(
    config: SetConfig<'_>,
    row: &UnsetNode,
    cell: &UnsetNode,
    column: usize,
    span: usize,
    stores: &Universe,
) -> Result<Node, ExecError> {
    let width = config.resolved.columns[column];
    let target = spanned_target(column, span, config.resolved, config.prototype, stores)?;
    let glue = cell_glue_setting(config.kind, cell, target)?;
    let fields = match config.kind {
        AlignmentKind::HAlign => BoxNodeFields {
            width,
            height: row.height,
            depth: row.depth,
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: glue.ratio,
            glue_sign: glue.sign,
            glue_order: glue.order,
            children: cell.children,
        },
        AlignmentKind::VAlign => BoxNodeFields {
            width: row.width,
            height: width,
            depth: cell.depth,
            shift: Scaled::from_raw(0),
            display: false,
            glue_set: glue.ratio,
            glue_sign: glue.sign,
            glue_order: glue.order,
            children: cell.children,
        },
    };
    Ok(match config.kind {
        AlignmentKind::HAlign => Node::HList(BoxNode::new(fields)),
        AlignmentKind::VAlign => Node::VList(BoxNode::new(fields)),
    })
}

fn spanned_target(
    column: usize,
    span: usize,
    resolved: &ResolvedWidths,
    prototype: &Prototype,
    stores: &Universe,
) -> Result<Scaled, ExecError> {
    let mut target = resolved.columns[column];
    for offset in 1..span {
        let spanned_column = column + offset;
        let glue = stores.glue(resolved.tabskips[spanned_column]);
        target = add_scaled(target, glue.width)?;
        target = add_scaled(target, glue_adjustment(glue, prototype)?)?;
        target = add_scaled(target, resolved.columns[spanned_column])?;
    }
    Ok(target)
}

fn glue_adjustment(
    glue: tex_state::glue::GlueSpec,
    prototype: &Prototype,
) -> Result<Scaled, ExecError> {
    match prototype.box_node.glue_sign {
        Sign::Stretching if glue.stretch_order == prototype.box_node.glue_order => {
            rounded_glue(prototype.box_node.glue_set, glue.stretch)
        }
        Sign::Shrinking if glue.shrink_order == prototype.box_node.glue_order => {
            rounded_glue(prototype.box_node.glue_set, glue.shrink)?
                .checked_neg()
                .ok_or(ExecError::ArithmeticOverflow)
        }
        _ => Ok(Scaled::from_raw(0)),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GlueSetting {
    ratio: GlueSetRatio,
    sign: Sign,
    order: Order,
}

fn cell_glue_setting(
    kind: AlignmentKind,
    cell: &UnsetNode,
    target: Scaled,
) -> Result<GlueSetting, ExecError> {
    let natural = unset_axis_size(kind, cell);
    let diff = i64::from(target.raw()) - i64::from(natural.raw());
    if diff == 0 {
        return Ok(GlueSetting {
            ratio: GlueSetRatio::ZERO,
            sign: Sign::Normal,
            order: Order::Normal,
        });
    }

    if diff > 0 {
        let excess = scaled_from_i64(diff)?;
        let ratio = if cell.stretch.raw() == 0 {
            GlueSetRatio::ZERO
        } else {
            GlueSetRatio::from_scaled_ratio(excess, cell.stretch)
        };
        return Ok(GlueSetting {
            ratio,
            sign: Sign::Stretching,
            order: cell.stretch_order,
        });
    }

    let excess = scaled_from_i64(-diff)?;
    let ratio = if cell.shrink.raw() == 0 {
        GlueSetRatio::ZERO
    } else if cell.shrink_order == Order::Normal && excess.raw() > cell.shrink.raw() {
        GlueSetRatio::UNITY
    } else {
        GlueSetRatio::from_scaled_ratio(excess, cell.shrink)
    };
    Ok(GlueSetting {
        ratio,
        sign: Sign::Shrinking,
        order: cell.shrink_order,
    })
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
            glue_order: Order::Normal,
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
            glue_order: Order::Normal,
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
    }
}

fn rounded_glue(ratio: GlueSetRatio, amount: Scaled) -> Result<Scaled, ExecError> {
    let product = i128::from(ratio.raw()) * i128::from(amount.raw());
    let rounded = rounded_div(product, i128::from(GLUE_SET_RATIO_SCALE));
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

#[cfg(debug_assertions)]
fn debug_assert_no_unset_nodes(stores: &Universe, nodes: &[Node]) {
    let mut stack = Vec::new();
    for node in nodes {
        debug_assert_no_unset_node(node, &mut stack);
    }
    while let Some(list) = stack.pop() {
        for node in stores.nodes(list) {
            debug_assert_no_unset_node(node, &mut stack);
        }
    }
}

#[cfg(not(debug_assertions))]
fn debug_assert_no_unset_nodes(_stores: &Universe, _nodes: &[Node]) {}

#[cfg(debug_assertions)]
fn debug_assert_no_unset_node(node: &Node, stack: &mut Vec<NodeListId>) {
    match node {
        Node::Unset(_) => panic!("unset node escaped fin_align"),
        Node::HList(box_node) | Node::VList(box_node) => stack.push(box_node.children),
        Node::Disc {
            pre, post, replace, ..
        } => {
            stack.push(*pre);
            stack.push(*post);
            stack.push(*replace);
        }
        Node::Ins { content, .. } | Node::Adjust(content) => stack.push(*content),
        Node::MathNoad(noad) => {
            debug_assert_math_field(&noad.nucleus, stack);
            debug_assert_math_field(&noad.subscript, stack);
            debug_assert_math_field(&noad.superscript, stack);
        }
        Node::FractionNoad(fraction) => {
            stack.push(fraction.numerator);
            stack.push(fraction.denominator);
        }
        Node::MathChoice(choice) => {
            stack.push(choice.display);
            stack.push(choice.text);
            stack.push(choice.script);
            stack.push(choice.script_script);
        }
        Node::MathList(list) => stack.push(list.content),
        Node::Char { .. }
        | Node::Lig { .. }
        | Node::Kern { .. }
        | Node::Glue { .. }
        | Node::Penalty(_)
        | Node::Rule { .. }
        | Node::Mark { .. }
        | Node::Whatsit(_)
        | Node::MathOn(_)
        | Node::MathOff(_)
        | Node::MathStyle(_)
        | Node::Nonscript => {}
    }
}

#[cfg(debug_assertions)]
fn debug_assert_math_field(field: &tex_state::math::MathField, stack: &mut Vec<NodeListId>) {
    match field {
        tex_state::math::MathField::SubBox(list) | tex_state::math::MathField::SubMlist(list) => {
            stack.push(*list)
        }
        tex_state::math::MathField::Empty
        | tex_state::math::MathField::MathChar(_)
        | tex_state::math::MathField::MathTextChar(_) => {}
    }
}
