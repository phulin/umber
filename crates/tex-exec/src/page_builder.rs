//! TeX.web page-builder accounting for outer vertical contributions.

use tex_state::Universe;
use tex_state::env::banks::GlueParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{GlueKind, Node};
use tex_state::page::{
    AWFUL_BAD, DEPLORABLE, EJECT_PENALTY, INF_PENALTY, PageContents, PageDimension,
};
use tex_state::scaled::Scaled;
use tex_typeset::{INF_BAD, badness};

use crate::ExecError;

pub(crate) fn build_page(stores: &mut Universe) -> Result<(), ExecError> {
    if stores.page_fire_up().is_some() {
        return Ok(());
    }

    while let Some(node) = stores.page_contribution_front().cloned() {
        stores.update_page_last_from_node(&node);
        match node {
            Node::HList(_) | Node::VList(_) | Node::Rule { .. } => {
                if !stores.page_contents().has_box() {
                    initialize_page_with_topskip(stores, &node)?;
                    continue;
                }
                prepare_box_or_rule(stores, &node)?;
                contribute_front(stores)?;
            }
            Node::Glue { spec, .. } => {
                if !stores.page_contents().has_box() {
                    discard_front(stores);
                } else if stores.current_page_tail().is_some_and(precedes_break) {
                    check_break(stores, 0)?;
                    if stores.page_fire_up().is_some() {
                        return Ok(());
                    }
                    update_glue_or_kern(stores, &node)?;
                    contribute_front(stores)?;
                } else {
                    let _ = spec;
                    update_glue_or_kern(stores, &node)?;
                    contribute_front(stores)?;
                }
            }
            Node::Kern { .. } => {
                if !stores.page_contents().has_box() {
                    discard_front(stores);
                } else if stores.page_contribution_second().is_none() {
                    return Ok(());
                } else if matches!(stores.page_contribution_second(), Some(Node::Glue { .. })) {
                    check_break(stores, 0)?;
                    if stores.page_fire_up().is_some() {
                        return Ok(());
                    }
                    update_glue_or_kern(stores, &node)?;
                    contribute_front(stores)?;
                } else {
                    update_glue_or_kern(stores, &node)?;
                    contribute_front(stores)?;
                }
            }
            Node::Penalty(penalty) => {
                if !stores.page_contents().has_box() {
                    discard_front(stores);
                } else {
                    check_break(stores, penalty)?;
                    if stores.page_fire_up().is_some() {
                        return Ok(());
                    }
                    contribute_front(stores)?;
                }
            }
            Node::Ins { .. } => {
                if stores.page_contents() == PageContents::Empty {
                    stores.freeze_page_specs(PageContents::InsertsOnly);
                }
                // TODO(umber2-4ci.5): account insertion classes, split costs,
                // and inserts-only page goal corrections here.
                contribute_front(stores)?;
            }
            Node::Whatsit(_) | Node::Mark { .. } => {
                contribute_front(stores)?;
            }
            Node::Char { .. }
            | Node::Lig { .. }
            | Node::Unset
            | Node::Disc { .. }
            | Node::MathOn
            | Node::MathOff
            | Node::Adjust(_) => {
                contribute_front(stores)?;
            }
        }
    }
    Ok(())
}

fn initialize_page_with_topskip(stores: &mut Universe, node: &Node) -> Result<(), ExecError> {
    if stores.page_contents() == PageContents::Empty {
        stores.freeze_page_specs(PageContents::BoxThere);
    } else {
        stores.set_page_contents(PageContents::BoxThere);
    }
    let top_skip = stores.glue(stores.glue_param(GlueParam::TOP_SKIP));
    let adjusted = GlueSpec {
        width: top_skip
            .width
            .checked_sub(vertical_height(node))
            .filter(|width| width.raw() > 0)
            .unwrap_or_else(|| Scaled::from_raw(0)),
        stretch: top_skip.stretch,
        stretch_order: top_skip.stretch_order,
        shrink: top_skip.shrink,
        shrink_order: top_skip.shrink_order,
    };
    let spec = stores.intern_glue(adjusted);
    stores.prepend_page_contribution(Node::Glue {
        spec,
        kind: GlueKind::Normal,
    });
    Ok(())
}

fn prepare_box_or_rule(stores: &mut Universe, node: &Node) -> Result<(), ExecError> {
    let total = add(
        stores.page_dimension(PageDimension::Total),
        stores.page_dimension(PageDimension::Depth),
    )?;
    let total = add(total, vertical_height(node))?;
    stores.set_page_dimension(PageDimension::Total, total);
    stores.set_page_dimension(PageDimension::Depth, vertical_depth(node));
    Ok(())
}

fn update_glue_or_kern(stores: &mut Universe, node: &Node) -> Result<(), ExecError> {
    let width = match node {
        Node::Kern { amount, .. } => *amount,
        Node::Glue { spec, .. } => {
            let spec = stores.glue(*spec);
            add_glue_stretch(stores, spec)?;
            let shrink = add(stores.page_dimension(PageDimension::Shrink), spec.shrink)?;
            stores.set_page_dimension(PageDimension::Shrink, shrink);
            spec.width
        }
        _ => return Ok(()),
    };
    let total = add(
        stores.page_dimension(PageDimension::Total),
        stores.page_dimension(PageDimension::Depth),
    )?;
    let total = add(total, width)?;
    stores.set_page_dimension(PageDimension::Total, total);
    stores.set_page_dimension(PageDimension::Depth, Scaled::from_raw(0));
    Ok(())
}

fn add_glue_stretch(stores: &mut Universe, spec: GlueSpec) -> Result<(), ExecError> {
    let dimension = match spec.stretch_order {
        Order::Normal => PageDimension::Stretch,
        Order::Fil => PageDimension::FilStretch,
        Order::Fill => PageDimension::FillStretch,
        Order::Filll => PageDimension::FilllStretch,
    };
    let value = add(stores.page_dimension(dimension), spec.stretch)?;
    stores.set_page_dimension(dimension, value);
    Ok(())
}

fn check_break(stores: &mut Universe, penalty: i32) -> Result<(), ExecError> {
    if penalty >= INF_PENALTY {
        return Ok(());
    }
    let badness = page_badness(stores)?;
    let mut cost = if badness < AWFUL_BAD {
        if penalty <= EJECT_PENALTY {
            penalty
        } else if badness < INF_BAD {
            badness
                .checked_add(penalty)
                .and_then(|value| value.checked_add(stores.insert_penalties()))
                .ok_or(ExecError::ArithmeticOverflow)?
        } else {
            DEPLORABLE
        }
    } else {
        badness
    };
    if stores.insert_penalties() >= INF_PENALTY {
        cost = AWFUL_BAD;
    }

    let break_index = stores.current_page_len();
    if cost <= stores.least_page_cost() {
        stores.record_best_page_break(
            break_index,
            stores.page_dimension(PageDimension::Goal),
            cost,
        );
    }
    if cost == AWFUL_BAD || penalty <= EJECT_PENALTY {
        stores.record_page_fire_up(break_index);
    }
    Ok(())
}

fn page_badness(stores: &Universe) -> Result<i32, ExecError> {
    let total = stores.page_dimension(PageDimension::Total);
    let goal = stores.page_dimension(PageDimension::Goal);
    if total < goal {
        if stores.page_dimension(PageDimension::FilStretch).raw() != 0
            || stores.page_dimension(PageDimension::FillStretch).raw() != 0
            || stores.page_dimension(PageDimension::FilllStretch).raw() != 0
        {
            Ok(0)
        } else {
            Ok(badness(
                sub(goal, total)?,
                stores.page_dimension(PageDimension::Stretch),
            ))
        }
    } else {
        let excess = sub(total, goal)?;
        if excess > stores.page_dimension(PageDimension::Shrink) {
            Ok(AWFUL_BAD)
        } else {
            Ok(badness(
                excess,
                stores.page_dimension(PageDimension::Shrink),
            ))
        }
    }
}

fn contribute_front(stores: &mut Universe) -> Result<(), ExecError> {
    ensure_max_depth(stores)?;
    if let Some(node) = stores.pop_page_contribution_front() {
        stores.push_current_page_node(node);
    }
    Ok(())
}

fn discard_front(stores: &mut Universe) {
    let _ = stores.pop_page_contribution_front();
}

fn ensure_max_depth(stores: &mut Universe) -> Result<(), ExecError> {
    let depth = stores.page_dimension(PageDimension::Depth);
    let max_depth = stores.page_max_depth();
    if depth > max_depth {
        let excess = sub(depth, max_depth)?;
        let total = add(stores.page_dimension(PageDimension::Total), excess)?;
        stores.set_page_dimension(PageDimension::Total, total);
        stores.set_page_dimension(PageDimension::Depth, max_depth);
    }
    Ok(())
}

fn precedes_break(node: &Node) -> bool {
    !matches!(
        node,
        Node::Glue { .. } | Node::Kern { .. } | Node::Penalty(_) | Node::MathOn | Node::MathOff
    )
}

fn vertical_height(node: &Node) -> Scaled {
    match node {
        Node::HList(box_node) | Node::VList(box_node) => box_node.height,
        Node::Rule { height, .. } => height.unwrap_or_else(|| Scaled::from_raw(0)),
        _ => Scaled::from_raw(0),
    }
}

fn vertical_depth(node: &Node) -> Scaled {
    match node {
        Node::HList(box_node) | Node::VList(box_node) => box_node.depth,
        Node::Rule { depth, .. } => depth.unwrap_or_else(|| Scaled::from_raw(0)),
        _ => Scaled::from_raw(0),
    }
}

fn add(lhs: Scaled, rhs: Scaled) -> Result<Scaled, ExecError> {
    lhs.checked_add(rhs).ok_or(ExecError::ArithmeticOverflow)
}

fn sub(lhs: Scaled, rhs: Scaled) -> Result<Scaled, ExecError> {
    lhs.checked_sub(rhs).ok_or(ExecError::ArithmeticOverflow)
}
