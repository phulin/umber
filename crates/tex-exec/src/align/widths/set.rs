use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
#[cfg(test)]
mod tests;

use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign, UnsetNode};
use tex_state::node_arena::PageListId;
use tex_state::scaled::{GlueSetRatio, Scaled};

use crate::ExecError;
use crate::mode::AlignmentKind;

use super::{
    Prototype, ResolvedWidths, add_scaled, empty_column_box, rounded_glue, scaled_from_i64,
    tabskip_node, unset_axis_size,
};

#[derive(Clone)]
struct SetConfig<'a> {
    kind: AlignmentKind,
    resolved: &'a ResolvedWidths,
    prototype: &'a Prototype,
    empty: PageListId,
    /// TeX82 §800's `o`: `display_indent` when the alignment is a display,
    /// zero otherwise. §807 shifts every row by it and §806 every rule.
    offset: Scaled,
}

#[allow(clippy::too_many_arguments)] // Alignment resolution applies one explicit immutable width plan.
pub(super) fn set_alignment_nodes<G>(
    kind: AlignmentKind,
    rows: PageListId,
    resolved: &ResolvedWidths,
    prototype: &Prototype,
    empty: PageListId,
    offset: Scaled,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
) -> Result<Vec<Node>, ExecError> {
    let config = SetConfig {
        kind,
        resolved,
        prototype,
        empty,
        offset,
    };
    let mut out = Vec::with_capacity(rows.len());
    for index in 0..rows.len() {
        let node = stores
            .page_node_list(rows)
            .expect("alignment rows belong to the live page arena")
            .nodes()
            .owned_node(index)
            .expect("alignment row index remains in range")
            .clone();
        match node {
            Node::Unset(row) => {
                let set = set_row(&config, &row, stores)?;
                out.push(set);
            }
            Node::Rule {
                width,
                height,
                depth,
            } => out.push(set_running_rule(
                &config,
                width,
                height,
                depth,
                stores,
                diagnostic_effects,
                geometry,
                diagnostic_context,
            )),
            _ => out.push(node),
        }
    }
    Ok(out)
}

/// TeX82 §806, `<Make the running dimensions in rule q extend to the
/// boundaries of the alignment>`.
///
/// The running dimensions come from the prototype box, and then -- this is the
/// half that was missing -- `if o<>0 then begin r:=link(q); link(q):=null;
/// q:=hpack(q,natural); shift_amount(q):=o; link(q):=r; link(s):=q end`. A rule
/// node has no `shift_amount` field of its own, so a display alignment can only
/// indent one by wrapping it in a box. §807 shifts rows by the same `o`, and
/// leaving the rule unwrapped left it starting at the margin while every row
/// beside it was indented (`umber2-jnfg`).
fn set_running_rule<G>(
    config: &SetConfig<'_>,
    width: Option<Scaled>,
    height: Option<Scaled>,
    depth: Option<Scaled>,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
) -> Node {
    let prototype = &config.prototype.box_node;
    // TeX82 §808 applies all three tests independently; alignment direction
    // affects how the prototype was packed, not which running dimensions are
    // resolved from it.
    let rule = Node::Rule {
        width: Some(width.unwrap_or(prototype.width)),
        height: Some(height.unwrap_or(prototype.height)),
        depth: Some(depth.unwrap_or(prototype.depth)),
    };
    if config.offset.raw() == 0 {
        return rule;
    }
    let list = stores.publish_page_nodes(vec![rule]);
    let mut packed = crate::packing_params::hpack(
        stores,
        diagnostic_effects,
        geometry,
        diagnostic_context,
        list,
        tex_typeset::PackSpec::Natural,
        crate::packing_params::hpack_params(stores),
    )
    .node;
    packed.shift = config.offset;
    Node::HList(packed)
}

fn set_row<G>(
    config: &SetConfig<'_>,
    row: &UnsetNode,
    stores: &mut CommandContext<'_, G>,
) -> Result<Node, ExecError> {
    let children = set_row_children(config, row, stores)?;
    let fields = match config.kind {
        AlignmentKind::HAlign => BoxNodeFields {
            width: config.prototype.box_node.width,
            height: row.height,
            depth: row.depth,
            shift: config.offset,
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: config.prototype.box_node.glue_set,
            glue_sign: config.prototype.box_node.glue_sign,
            glue_order: config.prototype.box_node.glue_order,
            children,
        },
        AlignmentKind::VAlign => BoxNodeFields {
            width: row.width,
            height: config.prototype.box_node.height,
            depth: row.depth,
            shift: config.offset,
            box_lr: tex_state::node::BoxLr::Normal,
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

fn set_row_children<G>(
    config: &SetConfig<'_>,
    row: &UnsetNode,
    stores: &mut CommandContext<'_, G>,
) -> Result<PageListId, ExecError> {
    let children = stores
        .page_node_list(row.children)
        .expect("alignment row belongs to the live page arena")
        .nodes();
    let mut replacements = Vec::new();
    let mut column = 0usize;
    for (index, child) in children.iter().enumerate() {
        if let Node::Unset(cell) = child {
            let span = usize::from(cell.span_count) + 1;
            let mut output = Vec::with_capacity(span.saturating_mul(2).saturating_sub(1));
            output.push(set_cell(config, row, cell, column, span, stores)?);
            for offset in 1..span {
                let spanned_column = column + offset;
                output.push(tabskip_node(config.resolved.tabskips[spanned_column]));
                output.push(empty_column_box(
                    config.kind,
                    config.resolved.columns[spanned_column],
                    config.empty,
                ));
            }
            column += span;
            replacements.push((index, output));
        }
    }
    let source_len = children.len();
    drop(children);

    let mut slices = Vec::new();
    let mut pieces = Vec::with_capacity(replacements.len().saturating_mul(2) + 1);
    let mut start = 0;
    for (index, output) in replacements {
        if start < index {
            pieces.push(stores.slice_page_node_sequence(row.children, start..index, &mut slices));
        }
        pieces.push(stores.publish_page_nodes(output));
        start = index + 1;
    }
    if start < source_len {
        pieces.push(stores.slice_page_node_sequence(row.children, start..source_len, &mut slices));
    }
    Ok(stores.compose_page_node_sequences(&pieces))
}

fn set_cell<G>(
    config: &SetConfig<'_>,
    row: &UnsetNode,
    cell: &UnsetNode<PageListId>,
    column: usize,
    span: usize,
    stores: &CommandContext<'_, G>,
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
            box_lr: tex_state::node::BoxLr::Normal,
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
            box_lr: tex_state::node::BoxLr::Normal,
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

fn spanned_target<G>(
    column: usize,
    span: usize,
    resolved: &ResolvedWidths,
    prototype: &Prototype,
    _stores: &CommandContext<'_, G>,
) -> Result<Scaled, ExecError> {
    let mut target = resolved.columns[column];
    for offset in 1..span {
        let spanned_column = column + offset;
        let glue = resolved.tabskips[spanned_column];
        target = add_scaled(target, glue.width)?;
        target = add_scaled(target, glue_adjustment(glue, prototype)?)?;
        target = add_scaled(target, resolved.columns[spanned_column])?;
    }
    Ok(target)
}

fn glue_adjustment(glue: GlueSpec, prototype: &Prototype) -> Result<Scaled, ExecError> {
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

fn cell_glue_setting<List>(
    kind: AlignmentKind,
    cell: &UnsetNode<List>,
    target: Scaled,
) -> Result<GlueSetting, ExecError> {
    let natural = unset_axis_size(kind, cell)?;
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
