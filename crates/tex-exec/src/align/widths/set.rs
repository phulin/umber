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

pub(super) fn set_alignment_nodes<G>(
    kind: AlignmentKind,
    rows: &[Node],
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
    for node in rows {
        match node {
            Node::Unset(row) => {
                let set = set_row(&config, row, stores)?;
                out.push(set);
            }
            Node::Rule { .. } => out.push(set_running_rule(
                &config,
                node,
                stores,
                diagnostic_effects,
                geometry,
                diagnostic_context,
            )),
            _ => out.push(node.clone()),
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
    node: &Node,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    geometry: &mut dyn crate::geometry::PackGeometrySink,
    diagnostic_context: &crate::pack_report::ExecutionDiagnosticContext,
) -> Node {
    let prototype = &config.prototype.box_node;
    let Node::Rule {
        width,
        height,
        depth,
    } = node
    else {
        return node.clone();
    };
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
    let children = stores.publish_page_nodes(children);
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
            children: children.clone(),
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
    stores: &CommandContext<'_, G>,
) -> Result<Vec<Node>, ExecError> {
    let mut out = Vec::new();
    let mut column = 0usize;
    for child in stores
        .page_node_list(row.children)
        .expect("alignment row belongs to the live page arena")
        .iter()
    {
        match child {
            tex_state::node_arena::NodeRef::Unset(cell) => {
                let span = usize::from(cell.span_count) + 1;
                out.push(set_cell(config, row, &cell, column, span, stores)?);
                for offset in 1..span {
                    let spanned_column = column + offset;
                    out.push(tabskip_node(config.resolved.tabskips[spanned_column]));
                    out.push(empty_column_box(
                        config.kind,
                        config.resolved.columns[spanned_column],
                        config.empty.clone(),
                    ));
                }
                column += span;
            }
            _ => out.push(child.to_owned_with(core::convert::identity)),
        }
    }
    Ok(out)
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
    stores: &CommandContext<'_, G>,
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
