use tex_state::glue::Order;
use tex_state::node::Node;
use tex_state::page::{AWFUL_BAD, DEPLORABLE, EJECT_PENALTY, INF_PENALTY};
use tex_state::scaled::Scaled;

use crate::{INF_BAD, TypesetState, badness};

/// Result of TeX's vertical break search.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerticalBreak {
    pub break_index: Option<usize>,
    pub best_height_plus_depth: Scaled,
}

/// Error produced by exact TeX scaled arithmetic in `vert_break`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerticalBreakError {
    ArithmeticOverflow,
}

/// TeX.web `vert_break`: choose the least-cost breakpoint in a vertical list.
pub fn vert_break(
    state: &impl TypesetState,
    nodes: &[Node],
    goal: Scaled,
    max_depth: Scaled,
) -> Result<VerticalBreak, VerticalBreakError> {
    let mut cur_height = Scaled::from_raw(0);
    let mut stretch = [Scaled::from_raw(0); 4];
    let mut shrink = Scaled::from_raw(0);
    let mut prev_depth = Scaled::from_raw(0);
    let mut least_cost = AWFUL_BAD;
    let mut best = VerticalBreak {
        break_index: None,
        best_height_plus_depth: Scaled::from_raw(0),
    };
    let mut prev_node = nodes.first();

    for index in 0..=nodes.len() {
        let node = nodes.get(index);
        let mut update_spacing = false;
        let mut penalty = None;

        match node {
            None => penalty = Some(EJECT_PENALTY),
            Some(Node::HList(box_node)) | Some(Node::VList(box_node)) => {
                cur_height = add(add(cur_height, prev_depth)?, box_node.height)?;
                prev_depth = box_node.depth;
            }
            Some(Node::Rule { height, depth, .. }) => {
                cur_height = add(
                    add(cur_height, prev_depth)?,
                    height.unwrap_or_else(|| Scaled::from_raw(0)),
                )?;
                prev_depth = depth.unwrap_or_else(|| Scaled::from_raw(0));
            }
            Some(Node::Glue { .. }) => {
                if prev_node.is_some_and(precedes_break) {
                    penalty = Some(0);
                    update_spacing = true;
                } else {
                    update_spacing_node(
                        state,
                        node,
                        &mut cur_height,
                        &mut prev_depth,
                        &mut stretch,
                        &mut shrink,
                    )?;
                }
            }
            Some(Node::Kern { .. }) => {
                if matches!(nodes.get(index + 1), Some(Node::Glue { .. })) {
                    penalty = Some(0);
                    update_spacing = true;
                } else {
                    update_spacing_node(
                        state,
                        node,
                        &mut cur_height,
                        &mut prev_depth,
                        &mut stretch,
                        &mut shrink,
                    )?;
                }
            }
            Some(Node::Penalty(value)) => penalty = Some(*value),
            Some(
                Node::Whatsit(_)
                | Node::Mark { .. }
                | Node::Ins { .. }
                | Node::Char { .. }
                | Node::Lig { .. }
                | Node::Unset
                | Node::Disc { .. }
                | Node::MathOn
                | Node::MathOff
                | Node::Adjust(_),
            ) => {}
        }

        if let Some(penalty) = penalty
            && penalty < INF_PENALTY
        {
            let mut cost = vertical_break_badness(goal, cur_height, stretch, shrink)?;
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
                    break_index: node.map(|_| index),
                    best_height_plus_depth: add(cur_height, prev_depth)?,
                };
            }
            if cost == AWFUL_BAD || penalty <= EJECT_PENALTY {
                break;
            }
        }

        if update_spacing {
            update_spacing_node(
                state,
                node,
                &mut cur_height,
                &mut prev_depth,
                &mut stretch,
                &mut shrink,
            )?;
        }

        if prev_depth > max_depth {
            cur_height = add(cur_height, sub(prev_depth, max_depth)?)?;
            prev_depth = max_depth;
        }
        if let Some(node) = node {
            prev_node = Some(node);
        }
    }

    Ok(best)
}

fn update_spacing_node(
    state: &impl TypesetState,
    node: Option<&Node>,
    cur_height: &mut Scaled,
    prev_depth: &mut Scaled,
    stretch: &mut [Scaled; 4],
    shrink: &mut Scaled,
) -> Result<(), VerticalBreakError> {
    let width = match node {
        Some(Node::Kern { amount, .. }) => *amount,
        Some(Node::Glue { spec, .. }) => {
            let spec = state.glue(*spec);
            let order = spec.stretch_order as usize;
            stretch[order] = add(stretch[order], spec.stretch)?;
            *shrink = add(*shrink, spec.shrink)?;
            spec.width
        }
        _ => return Ok(()),
    };
    *cur_height = add(add(*cur_height, *prev_depth)?, width)?;
    *prev_depth = Scaled::from_raw(0);
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

fn precedes_break(node: &Node) -> bool {
    !matches!(
        node,
        Node::Glue { .. } | Node::Kern { .. } | Node::Penalty(_) | Node::MathOn | Node::MathOff
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
    use tex_state::Universe;
    use tex_state::glue::{GlueSpec, Order};
    use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, KernKind, Sign};
    use tex_state::scaled::GlueSetRatio;

    fn sp(raw: i32) -> Scaled {
        Scaled::from_raw(raw)
    }

    fn hbox(universe: &mut Universe, height: i32, depth: i32) -> Node {
        let children = universe.freeze_node_list(&[]);
        Node::HList(BoxNode::new(BoxNodeFields {
            width: sp(10),
            height: sp(height),
            depth: sp(depth),
            shift: sp(0),
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        }))
    }

    #[test]
    fn forced_penalty_breaks_before_the_penalty_node() {
        let mut universe = Universe::new();
        let nodes = vec![hbox(&mut universe, 10, 3), Node::Penalty(EJECT_PENALTY)];

        let split = vert_break(&universe, &nodes, sp(100), sp(2)).expect("vertical break");

        assert_eq!(split.break_index, Some(1));
        assert_eq!(split.best_height_plus_depth, sp(13));
    }

    #[test]
    fn glue_break_uses_stretch_badness() {
        let mut universe = Universe::new();
        let glue = universe.intern_glue(GlueSpec {
            width: sp(1),
            stretch: sp(100),
            stretch_order: Order::Normal,
            shrink: sp(0),
            shrink_order: Order::Normal,
        });
        let nodes = vec![
            hbox(&mut universe, 10, 0),
            Node::Glue {
                spec: glue,
                kind: GlueKind::Normal,
            },
            hbox(&mut universe, 40, 0),
        ];

        let split = vert_break(&universe, &nodes, sp(12), sp(10)).expect("vertical break");

        assert_eq!(split.break_index, Some(1));
        assert_eq!(split.best_height_plus_depth, sp(10));
    }

    #[test]
    fn end_break_returns_none_for_whole_list() {
        let mut universe = Universe::new();
        let nodes = vec![hbox(&mut universe, 7, 5)];

        let split = vert_break(&universe, &nodes, sp(100), sp(2)).expect("vertical break");

        assert_eq!(split.break_index, None);
        assert_eq!(split.best_height_plus_depth, sp(12));
    }

    #[test]
    fn kern_before_glue_is_a_legal_break() {
        let mut universe = Universe::new();
        let glue = universe.intern_glue(GlueSpec {
            width: sp(3),
            stretch: sp(0),
            stretch_order: Order::Normal,
            shrink: sp(0),
            shrink_order: Order::Normal,
        });
        let nodes = vec![
            hbox(&mut universe, 10, 0),
            Node::Kern {
                amount: sp(2),
                kind: KernKind::Explicit,
            },
            Node::Glue {
                spec: glue,
                kind: GlueKind::Normal,
            },
            hbox(&mut universe, 10, 0),
        ];

        let split = vert_break(&universe, &nodes, sp(10), sp(10)).expect("vertical break");

        assert_eq!(split.break_index, Some(1));
    }
}
