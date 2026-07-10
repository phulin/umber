use tex_state::Universe;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::NodeListId;
use tex_state::node::{BoxNode, BoxNodeFields, Node, Sign, UnsetNode};
use tex_state::scaled::{GlueSetRatio, Scaled};

use crate::ExecError;
use crate::mode::AlignmentKind;

use super::{
    Prototype, ResolvedWidths, add_scaled, empty_column_box, rounded_glue, scaled_from_i64,
    tabskip_node, unset_axis_size,
};

#[derive(Clone, Copy)]
struct SetConfig<'a> {
    kind: AlignmentKind,
    resolved: &'a ResolvedWidths,
    prototype: &'a Prototype,
    empty: NodeListId,
}

pub(super) fn set_alignment_nodes(
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
    for node in rows {
        match node {
            Node::Unset(row) => {
                let set = set_row(config, row, stores)?;
                out.push(set);
            }
            _ => {
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
