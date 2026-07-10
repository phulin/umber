//! TeX.web page-builder accounting for outer vertical contributions.

use tex_state::Universe;
use tex_state::env::banks::GlueParam;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{GlueKind, Node};
use tex_state::page::{
    AWFUL_BAD, DEPLORABLE, EJECT_PENALTY, INF_PENALTY, PageContents, PageDimension, PageInsertion,
    PageInsertionStatus,
};
use tex_state::scaled::{Scaled, nx_plus_y, x_over_n};
use tex_typeset::{INF_BAD, VerticalBreakError, badness, vert_break};

use crate::{ExecError, diagnostics};

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
                    let node = update_glue_or_kern(stores, &node)?;
                    contribute_front_as(stores, node)?;
                } else {
                    let _ = spec;
                    let node = update_glue_or_kern(stores, &node)?;
                    contribute_front_as(stores, node)?;
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
                    let node = update_glue_or_kern(stores, &node)?;
                    contribute_front_as(stores, node)?;
                } else {
                    let node = update_glue_or_kern(stores, &node)?;
                    contribute_front_as(stores, node)?;
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
                let node = prepare_insertion(stores, &node)?.unwrap_or(node);
                contribute_front_as(stores, node)?;
            }
            Node::Whatsit(_) | Node::Mark { .. } => {
                contribute_front(stores)?;
            }
            Node::Char { .. }
            | Node::Lig { .. }
            | Node::Unset(_)
            | Node::Disc { .. }
            | Node::MathOn(_)
            | Node::MathOff(_)
            | Node::MathNoad(_)
            | Node::FractionNoad(_)
            | Node::MathStyle(_)
            | Node::MathChoice(_)
            | Node::MathList(_)
            | Node::Nonscript
            | Node::Adjust(_) => {
                contribute_front(stores)?;
            }
        }
    }
    Ok(())
}

fn prepare_insertion(stores: &mut Universe, node: &Node) -> Result<Option<Node>, ExecError> {
    let Node::Ins {
        class,
        size,
        split_max_depth,
        floating_penalty,
        content,
        ..
    } = node
    else {
        return Ok(None);
    };

    let mut insertion = match stores.page_insertion(*class) {
        Some(insertion) => insertion,
        None => create_page_insertion(stores, *class)?,
    };
    let mut replacement = None;

    match insertion.status() {
        PageInsertionStatus::SplitUp { .. } => {
            add_insert_penalty(stores, *floating_penalty);
        }
        PageInsertionStatus::Inserting => {
            let current_index = stores.current_page_len();
            insertion.set_last_ins_index(Some(current_index));
            let delta = insertion_delta(stores)?;
            let scaled_size = scaled_insertion_size(*size, stores.count(*class))?;
            if ((scaled_size.raw() <= 0) || scaled_size <= delta)
                && add(insertion.height(), *size)? <= stores.dimen(*class)
            {
                let goal = sub(stores.page_dimension(PageDimension::Goal), scaled_size)?;
                stores.set_page_dimension(PageDimension::Goal, goal);
                insertion.set_height(add(insertion.height(), *size)?);
            } else {
                replacement = split_page_insertion(
                    stores,
                    &mut insertion,
                    current_index,
                    node,
                    *content,
                    *split_max_depth,
                )?;
            }
        }
    }

    stores.upsert_page_insertion(insertion);
    Ok(replacement)
}

fn create_page_insertion(stores: &mut Universe, class: u16) -> Result<PageInsertion, ExecError> {
    let existing_height = insertion_box_size(stores, class)?;
    let insertion = PageInsertion::new(class, existing_height);
    let scaled_height = scaled_insertion_size(existing_height, stores.count(class))?;
    let skip = stores.glue(stores.skip(class));
    let goal = sub(stores.page_dimension(PageDimension::Goal), scaled_height)?;
    let goal = sub(goal, skip.width)?;
    stores.set_page_dimension(PageDimension::Goal, goal);
    add_glue_stretch(stores, skip)?;
    let shrink = add(stores.page_dimension(PageDimension::Shrink), skip.shrink)?;
    stores.set_page_dimension(PageDimension::Shrink, shrink);
    if skip.shrink_order != Order::Normal && skip.shrink.raw() != 0 {
        diagnostics::report_insertion_skip_infinite_shrinkage(stores, class);
    }
    Ok(insertion)
}

fn insertion_box_size(stores: &Universe, class: u16) -> Result<Scaled, ExecError> {
    let Some(list) = stores.box_reg(class) else {
        return Ok(Scaled::from_raw(0));
    };
    let Some(node) = stores.nodes(list).first() else {
        return Ok(Scaled::from_raw(0));
    };
    match node {
        tex_state::node_arena::NodeRef::VList(box_node) => add(box_node.height, box_node.depth),
        tex_state::node_arena::NodeRef::HList(_) => Err(ExecError::UnsupportedShipoutNode {
            node: "hbox insertion box",
        }),
        _ => Ok(Scaled::from_raw(0)),
    }
}

fn insertion_delta(stores: &Universe) -> Result<Scaled, ExecError> {
    let delta = sub(
        stores.page_dimension(PageDimension::Goal),
        stores.page_dimension(PageDimension::Total),
    )?;
    let delta = sub(delta, stores.page_dimension(PageDimension::Depth))?;
    add(delta, stores.page_dimension(PageDimension::Shrink))
}

fn split_page_insertion(
    stores: &mut Universe,
    insertion: &mut PageInsertion,
    current_index: usize,
    node: &Node,
    content: tex_state::ids::NodeListId,
    split_max_depth: Scaled,
) -> Result<Option<Node>, ExecError> {
    let class = insertion.class();
    let count = stores.count(class);
    let mut capacity = if count <= 0 {
        Scaled::MAX_DIMEN
    } else {
        let available = sub(
            sub(
                stores.page_dimension(PageDimension::Goal),
                stores.page_dimension(PageDimension::Total),
            )?,
            stores.page_dimension(PageDimension::Depth),
        )?;
        inverse_scaled_insertion_capacity(available, count)?
    };
    let remaining_cap = sub(stores.dimen(class), insertion.height())?;
    if capacity > remaining_cap {
        capacity = remaining_cap;
    }

    let mut content_nodes = stores.nodes(content).to_vec();
    let split = vert_break(stores, &content_nodes, capacity, split_max_depth)
        .map_err(vertical_break_error)?;
    let replacement = normalize_insert_content_shrink(
        stores,
        node,
        &mut content_nodes,
        &split.infinite_shrink_glue,
    );
    insertion.set_height(add(insertion.height(), split.best_height_plus_depth)?);
    let scaled_best = scaled_insertion_size(split.best_height_plus_depth, count)?;
    let goal = sub(stores.page_dimension(PageDimension::Goal), scaled_best)?;
    stores.set_page_dimension(PageDimension::Goal, goal);
    insertion.set_status(PageInsertionStatus::SplitUp {
        broken_ins_index: current_index,
        broken_at: split.break_index,
    });

    match split.break_index {
        None => add_insert_penalty(stores, EJECT_PENALTY),
        Some(index) => {
            if let Some(Node::Penalty(penalty)) = content_nodes.get(index) {
                add_insert_penalty(stores, *penalty);
            }
        }
    }
    Ok(replacement)
}

fn add_insert_penalty(stores: &mut Universe, penalty: i32) {
    let value = stores.insert_penalties().saturating_add(penalty);
    stores.set_page_integer(tex_state::page::PageInteger::InsertPenalties, value);
}

fn scaled_insertion_size(size: Scaled, count: i32) -> Result<Scaled, ExecError> {
    if count == 1000 {
        return Ok(size);
    }
    let quotient = x_over_n(size, 1000)
        .map_err(|_| ExecError::ArithmeticOverflow)?
        .quotient;
    nx_plus_y(count, quotient, Scaled::from_raw(0)).map_err(|_| ExecError::ArithmeticOverflow)
}

fn inverse_scaled_insertion_capacity(size: Scaled, count: i32) -> Result<Scaled, ExecError> {
    if count == 1000 {
        return Ok(size);
    }
    let quotient = x_over_n(size, count)
        .map_err(|_| ExecError::ArithmeticOverflow)?
        .quotient;
    nx_plus_y(1000, quotient, Scaled::from_raw(0)).map_err(|_| ExecError::ArithmeticOverflow)
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
        kind: GlueKind::TopSkip,
        leader: None,
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

fn update_glue_or_kern(stores: &mut Universe, node: &Node) -> Result<Node, ExecError> {
    let mut replacement = None;
    let width = match node {
        Node::Kern { amount, .. } => *amount,
        Node::Glue { spec, kind, leader } => {
            let spec = stores.glue(*spec);
            let spec = finite_page_shrink(stores, spec);
            let finite_id = stores.intern_glue(spec);
            replacement = Some(Node::Glue {
                spec: finite_id,
                kind: *kind,
                leader: *leader,
            });
            add_glue_stretch(stores, spec)?;
            let shrink = add(stores.page_dimension(PageDimension::Shrink), spec.shrink)?;
            stores.set_page_dimension(PageDimension::Shrink, shrink);
            spec.width
        }
        _ => return Ok(node.clone()),
    };
    let total = add(
        stores.page_dimension(PageDimension::Total),
        stores.page_dimension(PageDimension::Depth),
    )?;
    let total = add(total, width)?;
    stores.set_page_dimension(PageDimension::Total, total);
    stores.set_page_dimension(PageDimension::Depth, Scaled::from_raw(0));
    Ok(replacement.unwrap_or_else(|| node.clone()))
}

fn finite_page_shrink(stores: &mut Universe, mut spec: GlueSpec) -> GlueSpec {
    if spec.shrink_order != Order::Normal && spec.shrink.raw() != 0 {
        diagnostics::report_page_infinite_shrinkage(stores);
        spec.shrink_order = Order::Normal;
    }
    spec
}

fn normalize_insert_content_shrink(
    stores: &mut Universe,
    insert_node: &Node,
    content_nodes: &mut [Node],
    indices: &[usize],
) -> Option<Node> {
    if indices.is_empty() {
        return None;
    }

    let mut changed = false;
    for &index in indices {
        let Some(Node::Glue { spec, kind, leader }) = content_nodes.get(index) else {
            continue;
        };
        let mut finite = stores.glue(*spec);
        if finite.shrink_order == Order::Normal || finite.shrink.raw() == 0 {
            continue;
        }
        diagnostics::report_split_infinite_shrinkage(stores);
        finite.shrink_order = Order::Normal;
        content_nodes[index] = Node::Glue {
            spec: stores.intern_glue(finite),
            kind: *kind,
            leader: *leader,
        };
        changed = true;
    }

    if !changed {
        return None;
    }
    let Node::Ins {
        class,
        size,
        split_top_skip,
        split_max_depth,
        floating_penalty,
        ..
    } = insert_node
    else {
        return None;
    };
    Some(Node::Ins {
        class: *class,
        size: *size,
        split_top_skip: *split_top_skip,
        split_max_depth: *split_max_depth,
        floating_penalty: *floating_penalty,
        content: stores.freeze_node_list(content_nodes),
    })
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

fn contribute_front_as(stores: &mut Universe, node: Node) -> Result<(), ExecError> {
    ensure_max_depth(stores)?;
    if stores.pop_page_contribution_front().is_some() {
        stores.update_page_last_from_node(&node);
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
        Node::Glue { .. }
            | Node::Kern { .. }
            | Node::Penalty(_)
            | Node::MathOn(_)
            | Node::MathOff(_)
    )
}

fn vertical_height(node: &Node) -> Scaled {
    match node {
        Node::HList(box_node) | Node::VList(box_node) => box_node.height,
        Node::Rule { height, .. } => height.unwrap_or_else(|| Scaled::from_raw(0)),
        Node::MathNoad(_)
        | Node::FractionNoad(_)
        | Node::MathStyle(_)
        | Node::MathChoice(_)
        | Node::MathList(_)
        | Node::Nonscript => Scaled::from_raw(0),
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

fn vertical_break_error(error: VerticalBreakError) -> ExecError {
    match error {
        VerticalBreakError::ArithmeticOverflow => ExecError::ArithmeticOverflow,
    }
}
