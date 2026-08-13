use tex_state::glue::GlueSpec;
use tex_state::node::{Direction, GlueKind, KernKind, Node};

use crate::TypesetState;

use super::{BreakDecision, BrokenLine, MaterializationAction, ParagraphTape, PostLineBreakParams};

pub fn post_line_break<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    breaks: &[BreakDecision],
    params: PostLineBreakParams,
) -> Vec<BrokenLine> {
    post_line_break_owned(state, nodes.to_vec(), breaks, params)
}

/// Stateful source-order materialization of a broken paragraph.
///
/// `materialize_next` accepts ownership of the previous line's node buffer,
/// clears it, and fills it with the next line. Callers that consume one line
/// before requesting another therefore pay for line storage only once.
pub struct LineMaterializer {
    semantic: ChannelCursor,
    physical: ChannelCursor,
    physical_breaks: Vec<BreakDecision>,
    actions: Vec<MaterializationAction>,
    breaks: Vec<BreakDecision>,
    line_no: usize,
    params: PostLineBreakParams,
}

struct ChannelCursor {
    nodes: std::vec::IntoIter<Node>,
    position: usize,
    node_count: usize,
    pending_post: Vec<Node>,
    active_directions: Vec<Direction>,
}

impl LineMaterializer {
    #[must_use]
    pub fn new(
        tape: ParagraphTape,
        breaks: Vec<BreakDecision>,
        params: PostLineBreakParams,
    ) -> Self {
        let ParagraphTape {
            sequence,
            materialization,
            ..
        } = tape;
        let (semantic, physical, boundaries) = sequence.into_parts();
        let physical_breaks = breaks
            .iter()
            .map(|decision| BreakDecision {
                position: boundaries[decision.position.min(boundaries.len() - 1)],
                ..*decision
            })
            .collect();
        Self {
            semantic: ChannelCursor::new(semantic),
            physical: ChannelCursor::new(physical),
            physical_breaks,
            actions: materialization,
            breaks,
            line_no: 0,
            params,
        }
    }

    pub fn from_nodes(
        nodes: Vec<Node>,
        breaks: Vec<BreakDecision>,
        params: PostLineBreakParams,
    ) -> Self {
        let mut actions: Vec<_> = nodes
            .iter()
            .map(|node| match node {
                Node::Disc { .. } => MaterializationAction::Discretionary,
                _ => MaterializationAction::Copy,
            })
            .collect();
        for decision in &breaks {
            if decision.position >= nodes.len() {
                continue;
            }
            if let Some((node, action)) = decision
                .position
                .checked_sub(1)
                .and_then(|index| nodes.get(index).zip(actions.get_mut(index)))
            {
                *action = match node {
                    Node::Glue { .. } => MaterializationAction::BreakDiscardable,
                    Node::MathOff(_) => MaterializationAction::BreakMath,
                    _ => *action,
                };
            }
        }
        Self {
            physical: ChannelCursor::new(nodes.clone()),
            semantic: ChannelCursor::new(nodes),
            physical_breaks: breaks.clone(),
            actions,
            breaks,
            line_no: 0,
            params,
        }
    }

    pub fn materialize_next<S: TypesetState>(
        &mut self,
        state: &S,
        mut line: Vec<Node>,
    ) -> Option<BrokenLine> {
        let decision = *self.breaks.get(self.line_no)?;
        let dimensions = self.params.shape.dimensions(self.line_no + 1);
        materialize_channel(
            state,
            &mut self.semantic,
            &decision,
            &self.params,
            Some(&self.actions),
            &mut line,
        );
        let mut physical_nodes = Vec::new();
        materialize_channel(
            state,
            &mut self.physical,
            &self.physical_breaks[self.line_no],
            &self.params,
            None,
            &mut physical_nodes,
        );

        let penalty_after = line_penalty_after(
            self.line_no,
            &self.breaks,
            decision.hyphenated,
            &self.params,
        );
        self.line_no += 1;
        Some(BrokenLine {
            physical_nodes,
            nodes: line,
            penalty_after,
            hyphenated: decision.hyphenated,
            dimensions,
        })
    }
}

impl ChannelCursor {
    fn new(nodes: Vec<Node>) -> Self {
        let node_count = nodes.len();
        Self {
            nodes: nodes.into_iter(),
            position: 0,
            node_count,
            pending_post: Vec::new(),
            active_directions: Vec::new(),
        }
    }
}

fn materialize_channel<S: TypesetState>(
    state: &S,
    cursor: &mut ChannelCursor,
    decision: &BreakDecision,
    params: &PostLineBreakParams,
    actions: Option<&[MaterializationAction]>,
    line: &mut Vec<Node>,
) {
    let end = decision.position.min(cursor.node_count);
    let start = cursor.position.min(end);
    let required = end
        .checked_sub(start)
        .and_then(|len| len.checked_add(cursor.pending_post.len()))
        .and_then(|len| len.checked_add(2))
        .expect("materialized line capacity fits usize");
    line.clear();
    line.reserve(required);
    if params.left_skip.spec() != GlueSpec::ZERO {
        line.push(Node::Glue {
            spec: params.left_skip.clone(),
            kind: GlueKind::LeftSkip,
            leader: None,
        });
    }
    line.extend(
        cursor
            .active_directions
            .iter()
            .copied()
            .map(Node::Direction),
    );
    let directional_start = line.len();
    line.append(&mut cursor.pending_post);
    cursor.pending_post = push_owned_line_segment(
        state,
        (&mut cursor.nodes, &mut cursor.position, cursor.node_count),
        end,
        decision,
        params.empty_list,
        actions,
        line,
    );
    update_active_directions(&line[directional_start..], &mut cursor.active_directions);
    line.extend(
        cursor
            .active_directions
            .iter()
            .rev()
            .copied()
            .map(|direction| Node::Direction(matching_end(direction))),
    );
    line.push(Node::Glue {
        spec: params.right_skip.clone(),
        kind: GlueKind::RightSkip,
        leader: None,
    });
    while cursor.nodes.as_slice().first().is_some_and(is_discardable) {
        let _ = cursor.nodes.next();
        cursor.position += 1;
    }
}

fn update_active_directions(nodes: &[Node], active: &mut Vec<Direction>) {
    for node in nodes {
        match node {
            Node::Direction(direction @ (Direction::BeginL | Direction::BeginR)) => {
                active.push(*direction);
            }
            Node::Direction(Direction::EndL) if active.last() == Some(&Direction::BeginL) => {
                let _ = active.pop();
            }
            Node::Direction(Direction::EndR) if active.last() == Some(&Direction::BeginR) => {
                let _ = active.pop();
            }
            _ => {}
        }
    }
}

const fn matching_end(direction: Direction) -> Direction {
    match direction {
        Direction::BeginM => Direction::EndM,
        Direction::BeginL => Direction::EndL,
        Direction::BeginR => Direction::EndR,
        Direction::EndM | Direction::EndL | Direction::EndR => direction,
    }
}

/// Materializes broken lines by moving nodes out of an owned paragraph.
///
/// The borrowed convenience entry point above remains useful to pure callers,
/// while execution can use this path to avoid cloning the entire paragraph a
/// second time after line breaking.
pub fn post_line_break_owned<S: TypesetState>(
    state: &S,
    nodes: Vec<Node>,
    breaks: &[BreakDecision],
    params: PostLineBreakParams,
) -> Vec<BrokenLine> {
    let mut lines = Vec::with_capacity(breaks.len());
    let mut materializer = LineMaterializer::from_nodes(nodes, breaks.to_vec(), params);
    while let Some(line) = materializer.materialize_next(state, Vec::new()) {
        lines.push(line);
    }
    lines
}

fn push_owned_line_segment<S: TypesetState>(
    state: &S,
    source: (&mut std::vec::IntoIter<Node>, &mut usize, usize),
    end: usize,
    decision: &BreakDecision,
    empty_list: tex_state::ids::NodeListId,
    actions: Option<&[MaterializationAction]>,
    out: &mut Vec<Node>,
) -> Vec<Node> {
    let (nodes, position, node_count) = source;
    let mut post = Vec::new();
    while *position < end {
        let absolute = *position;
        let node = nodes.next().expect("paragraph break position is in bounds");
        *position += 1;
        let action = actions.and_then(|actions| actions.get(absolute)).copied();
        match node {
            Node::Disc {
                kind,
                pre,
                post: post_list,
                ..
            } if decision.hyphenated && absolute + 1 == end => {
                // TeX82 §§879--882 makes the chosen discretionary compulsory
                // but retains the emptied node before its transplanted
                // pre-break material.
                out.push(Node::Disc {
                    kind,
                    pre: empty_list,
                    post: empty_list,
                    replace: empty_list,
                    physical_replace_count: 0,
                });
                out.extend(state.nodes(pre).into_iter().map(|node| node.to_owned()));
                post.extend(
                    state
                        .nodes(post_list)
                        .into_iter()
                        .map(|node| node.to_owned()),
                );
            }
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => {
                out.push(Node::Disc {
                    kind,
                    pre,
                    post,
                    replace,
                    physical_replace_count,
                });
                out.extend(state.nodes(replace).into_iter().map(|node| node.to_owned()));
            }
            Node::Glue { .. }
                if absolute + 1 == end
                    && end < node_count
                    && action
                        .is_none_or(|action| action == MaterializationAction::BreakDiscardable) => {
            }
            Node::MathOff(_)
                if absolute + 1 == end
                    && end < node_count
                    && action.is_none_or(|action| action == MaterializationAction::BreakMath) =>
            {
                out.push(Node::MathOff(tex_state::scaled::Scaled::from_raw(0)));
            }
            node => out.push(node),
        }
    }
    post
}

pub(super) fn line_penalty_after(
    line_no: usize,
    breaks: &[BreakDecision],
    hyphenated: bool,
    params: &PostLineBreakParams,
) -> Option<i32> {
    if line_no + 1 >= breaks.len() {
        return None;
    }
    let current_line = params.prev_graf.max(0) as usize + line_no + 1;
    let mut penalty = penalty_array_value(&params.interline_penalties, current_line)
        .unwrap_or(params.interline_penalty);
    penalty =
        penalty
            .checked_add(
                penalty_array_value(&params.club_penalties, line_no + 1)
                    .unwrap_or(if line_no == 0 { params.club_penalty } else { 0 }),
            )
            .expect("interline and club penalties fit TeX integer range");
    let lines_from_end = breaks.len() - line_no - 1;
    let widow_penalties = match params.widow_penalties.selector {
        super::WidowPenaltySelector::Ordinary => &params.widow_penalties.ordinary,
        super::WidowPenaltySelector::DisplayInterrupted => &params.widow_penalties.display,
    };
    penalty = penalty
        .checked_add(
            penalty_array_value(&widow_penalties.values, lines_from_end).unwrap_or(
                if line_no + 2 == breaks.len() {
                    widow_penalties.fallback
                } else {
                    0
                },
            ),
        )
        .expect("interline and widow penalties fit TeX integer range");
    if hyphenated {
        penalty = penalty
            .checked_add(params.broken_penalty)
            .expect("broken-line penalty fits TeX integer range");
    }
    (penalty != 0).then_some(penalty)
}

fn penalty_array_value(values: &[i32], one_based_index: usize) -> Option<i32> {
    (!values.is_empty()).then(|| values[one_based_index.min(values.len()) - 1])
}

fn is_discardable(node: &Node) -> bool {
    matches!(
        node,
        Node::Glue { .. }
            | Node::Kern {
                kind: KernKind::Explicit | KernKind::Mu,
                ..
            }
            | Node::Penalty(_)
            | Node::MathOn(_)
            | Node::MathOff(_)
    )
}
