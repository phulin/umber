use tex_state::glue::Order;
#[cfg(test)]
use tex_state::node::Node;
use tex_state::node_arena::{NodeCursor, NodeView};
use tex_state::page::{AWFUL_BAD, DEPLORABLE, EJECT_PENALTY, INF_PENALTY};
use tex_state::scaled::Scaled;

use crate::metrics::{ListMetrics, MetricEvent};
use crate::{INF_BAD, TypesetState, badness};

/// Result of TeX's vertical break search.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerticalBreak {
    pub break_index: Option<usize>,
    pub best_height_plus_depth: Scaled,
    pub infinite_shrink_glue: Vec<usize>,
}

/// Error produced by exact TeX scaled arithmetic in `vert_break`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerticalBreakError {
    ArithmeticOverflow,
}

/// TeX.web `vert_break`: choose the least-cost breakpoint in a vertical list.
pub fn vert_break(
    state: &impl TypesetState,
    nodes: NodeCursor<'_>,
    goal: Scaled,
    max_depth: Scaled,
) -> Result<VerticalBreak, VerticalBreakError> {
    let mut acc = VerticalBreakAccum::new();
    let mut least_cost = AWFUL_BAD;
    let mut best = VerticalBreak {
        break_index: None,
        best_height_plus_depth: Scaled::from_raw(0),
        infinite_shrink_glue: Vec::new(),
    };
    let mut prev_node = nodes.first();

    for index in 0..=nodes.len() {
        let node = nodes.get(index);
        let mut update_spacing = false;
        let mut penalty = None;

        match node {
            None => penalty = Some(EJECT_PENALTY),
            Some(NodeView::HList(box_node)) | Some(NodeView::VList(box_node)) => {
                // Vertical breaking accounts only for the box's vertical
                // extent. Perpendicular width and shift belong to packing and
                // must not introduce an otherwise impossible overflow here.
                acc.try_observe_vertical(MetricEvent::Box {
                    width: Scaled::from_raw(0),
                    height: box_node.height,
                    depth: box_node.depth,
                    shift: Scaled::from_raw(0),
                })
                .ok_or(VerticalBreakError::ArithmeticOverflow)?;
            }
            Some(NodeView::Rule { height, depth, .. }) => {
                acc.try_observe_vertical(MetricEvent::Rule {
                    width: Scaled::from_raw(0),
                    height: height.unwrap_or_else(|| Scaled::from_raw(0)),
                    depth: depth.unwrap_or_else(|| Scaled::from_raw(0)),
                })
                .ok_or(VerticalBreakError::ArithmeticOverflow)?;
            }
            Some(NodeView::Glue { .. }) => {
                if prev_node.as_ref().is_some_and(precedes_break) {
                    penalty = Some(0);
                    update_spacing = true;
                } else {
                    update_spacing_node(state, node.clone(), &mut acc, index)?;
                }
            }
            Some(NodeView::Kern { .. } | NodeView::MarginKern { .. }) => {
                if matches!(nodes.get(index + 1), Some(NodeView::Glue { .. })) {
                    penalty = Some(0);
                    update_spacing = true;
                } else {
                    update_spacing_node(state, node.clone(), &mut acc, index)?;
                }
            }
            Some(NodeView::Penalty(value)) => penalty = Some(value),
            Some(
                NodeView::Whatsit(_)
                | NodeView::Mark { .. }
                | NodeView::Ins { .. }
                | NodeView::Char { .. }
                | NodeView::Lig { .. }
                | NodeView::Unset(_)
                | NodeView::Disc { .. }
                | NodeView::MathOn(_)
                | NodeView::MathOff(_)
                | NodeView::Direction(_)
                | NodeView::MathNoad(_)
                | NodeView::FractionNoad(_)
                | NodeView::MathStyle(_)
                | NodeView::MathChoice(_)
                | NodeView::MathList(_)
                | NodeView::Nonscript
                | NodeView::Adjust(_),
            ) => {}
        }

        if let Some(penalty) = penalty
            && penalty < INF_PENALTY
        {
            let mut cost = vertical_break_badness(goal, acc.height, acc.stretch, acc.shrink)?;
            if cost < AWFUL_BAD {
                if penalty <= EJECT_PENALTY {
                    cost = penalty;
                } else if cost < INF_BAD {
                    cost = cost
                        .checked_add(penalty)
                        .ok_or(VerticalBreakError::ArithmeticOverflow)?;
                } else {
                    cost = DEPLORABLE;
                }
            }
            if cost <= least_cost {
                least_cost = cost;
                best = VerticalBreak {
                    break_index: node.as_ref().map(|_| index),
                    best_height_plus_depth: add(acc.height, acc.depth)?,
                    infinite_shrink_glue: Vec::new(),
                };
            }
            if cost == AWFUL_BAD || penalty <= EJECT_PENALTY {
                break;
            }
        }

        if update_spacing {
            update_spacing_node(state, node.clone(), &mut acc, index)?;
        }

        if acc.depth > max_depth {
            acc.height = add(acc.height, sub(acc.depth, max_depth)?)?;
            acc.depth = max_depth;
        }
        if let Some(node) = node {
            prev_node = Some(node);
        }
    }

    best.infinite_shrink_glue = acc.infinite_shrink_glue;
    Ok(best)
}

struct VerticalBreakAccum {
    metrics: ListMetrics,
    stretch: [Scaled; 4],
    shrink: Scaled,
    infinite_shrink_glue: Vec<usize>,
}

impl VerticalBreakAccum {
    fn new() -> Self {
        Self {
            metrics: ListMetrics::ZERO,
            stretch: [Scaled::from_raw(0); 4],
            shrink: Scaled::from_raw(0),
            infinite_shrink_glue: Vec::new(),
        }
    }
}

impl std::ops::Deref for VerticalBreakAccum {
    type Target = ListMetrics;

    fn deref(&self) -> &Self::Target {
        &self.metrics
    }
}

impl std::ops::DerefMut for VerticalBreakAccum {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.metrics
    }
}

fn update_spacing_node(
    _state: &impl TypesetState,
    node: Option<NodeView<'_>>,
    acc: &mut VerticalBreakAccum,
    index: usize,
) -> Result<(), VerticalBreakError> {
    let width = match node {
        Some(NodeView::Kern { amount, .. }) => amount,
        Some(NodeView::Glue { spec, .. }) => {
            let order = spec.stretch_order as usize;
            acc.stretch[order] = add(acc.stretch[order], spec.stretch)?;
            acc.shrink = add(acc.shrink, spec.shrink)?;
            if spec.shrink_order != Order::Normal && spec.shrink.raw() != 0 {
                acc.infinite_shrink_glue.push(index);
            }
            spec.width
        }
        _ => return Ok(()),
    };
    // Preserve vert_break's checked-arithmetic order: unlike packing's
    // vertical measurement, this domain adds the saved depth to the running
    // height before adding the spacing node.
    acc.height = add(add(acc.height, acc.depth)?, width)?;
    acc.depth = Scaled::from_raw(0);
    Ok(())
}

fn vertical_break_badness(
    goal: Scaled,
    cur_height: Scaled,
    stretch: [Scaled; 4],
    shrink: Scaled,
) -> Result<i32, VerticalBreakError> {
    if cur_height < goal {
        if stretch[Order::Fil as usize].raw() != 0
            || stretch[Order::Fill as usize].raw() != 0
            || stretch[Order::Filll as usize].raw() != 0
        {
            Ok(0)
        } else {
            Ok(badness(
                sub(goal, cur_height)?,
                stretch[Order::Normal as usize],
            ))
        }
    } else if sub(cur_height, goal)? > shrink {
        Ok(AWFUL_BAD)
    } else {
        Ok(badness(sub(cur_height, goal)?, shrink))
    }
}

fn precedes_break(node: &NodeView<'_>) -> bool {
    !matches!(
        node,
        NodeView::Glue { .. }
            | NodeView::Kern { .. }
            | NodeView::Penalty(_)
            | NodeView::MathOn(_)
            | NodeView::MathOff(_)
    )
}

fn add(lhs: Scaled, rhs: Scaled) -> Result<Scaled, VerticalBreakError> {
    lhs.checked_add(rhs)
        .ok_or(VerticalBreakError::ArithmeticOverflow)
}

fn sub(lhs: Scaled, rhs: Scaled) -> Result<Scaled, VerticalBreakError> {
    lhs.checked_sub(rhs)
        .ok_or(VerticalBreakError::ArithmeticOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_state::TestState;
    use tex_state::glue::{GlueSpec, Order};
    use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, KernKind, Sign};
    use tex_state::scaled::GlueSetRatio;

    mod planned;

    fn sp(raw: i32) -> Scaled {
        Scaled::from_raw(raw)
    }

    fn hbox(universe: &mut TestState, height: i32, depth: i32) -> Node {
        let children = universe.publish_page_nodes(&[]);
        Node::HList(BoxNode::new(BoxNodeFields {
            width: sp(10),
            height: sp(height),
            depth: sp(depth),
            shift: sp(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        }))
    }

    #[test]
    fn forced_penalty_breaks_before_the_penalty_node() {
        let mut universe = TestState::new();
        let nodes = vec![hbox(&mut universe, 10, 3), Node::Penalty(EJECT_PENALTY)];

        let split = vert_break(&universe, NodeCursor::owned(&nodes), sp(100), sp(2))
            .expect("vertical break");

        assert_eq!(split.break_index, Some(1));
        assert_eq!(split.best_height_plus_depth, sp(13));
    }

    #[test]
    fn glue_break_uses_stretch_badness() {
        let mut universe = TestState::new();
        let glue = GlueSpec {
            width: sp(1),
            stretch: sp(100),
            stretch_order: Order::Normal,
            shrink: sp(0),
            shrink_order: Order::Normal,
        };
        let nodes = vec![
            hbox(&mut universe, 10, 0),
            Node::Glue {
                spec: glue,
                kind: GlueKind::Normal,

                leader: None,
            },
            hbox(&mut universe, 40, 0),
        ];

        let split = vert_break(&universe, NodeCursor::owned(&nodes), sp(12), sp(10))
            .expect("vertical break");

        assert_eq!(split.break_index, Some(1));
        assert_eq!(split.best_height_plus_depth, sp(10));
    }

    #[test]
    fn end_break_returns_none_for_whole_list() {
        let mut universe = TestState::new();
        let nodes = vec![hbox(&mut universe, 7, 5)];

        let split = vert_break(&universe, NodeCursor::owned(&nodes), sp(100), sp(2))
            .expect("vertical break");

        assert_eq!(split.break_index, None);
        assert_eq!(split.best_height_plus_depth, sp(12));
    }

    #[test]
    fn kern_before_glue_is_a_legal_break() {
        let mut universe = TestState::new();
        let glue = GlueSpec {
            width: sp(3),
            stretch: sp(0),
            stretch_order: Order::Normal,
            shrink: sp(0),
            shrink_order: Order::Normal,
        };
        let nodes = vec![
            hbox(&mut universe, 10, 0),
            Node::Kern {
                amount: sp(2),
                kind: KernKind::Explicit,
            },
            Node::Glue {
                spec: glue,
                kind: GlueKind::Normal,

                leader: None,
            },
            hbox(&mut universe, 10, 0),
        ];

        let split = vert_break(&universe, NodeCursor::owned(&nodes), sp(10), sp(10))
            .expect("vertical break");

        assert_eq!(split.break_index, Some(1));
    }

    #[test]
    fn reports_infinite_shrink_glue_that_enters_accounting() {
        let mut universe = TestState::new();
        let glue = GlueSpec {
            width: sp(0),
            stretch: sp(0),
            stretch_order: Order::Normal,
            shrink: sp(5),
            shrink_order: Order::Fil,
        };
        let nodes = vec![
            hbox(&mut universe, 10, 0),
            Node::Glue {
                spec: glue,
                kind: GlueKind::Normal,

                leader: None,
            },
            Node::Penalty(0),
        ];

        let split = vert_break(&universe, NodeCursor::owned(&nodes), sp(12), sp(10))
            .expect("vertical break");

        assert_eq!(split.infinite_shrink_glue, vec![1]);
    }
}
