use tex_state::glue::GlueSpec;
use tex_state::node::{Direction, GlueKind, KernKind, Node};
use tex_state::node_sequence::{DirectHighCellLineage, DirectHighCellLineages, FrozenListRole};

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
    high_cell_lineages: std::vec::IntoIter<DirectHighCellLineages>,
    position: usize,
    node_count: usize,
    pending_post: Vec<Node>,
    pending_post_high_cell_lineages: Vec<DirectHighCellLineage>,
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
        let semantic_high_cell_lineages = sequence.semantic_high_cell_lineages().to_vec();
        let physical_high_cell_lineages = sequence.physical_high_cell_lineages().to_vec();
        let (semantic, physical, boundaries) = sequence.into_parts();
        let physical_breaks = breaks
            .iter()
            .map(|decision| BreakDecision {
                position: boundaries[decision.position.min(boundaries.len() - 1)],
                ..*decision
            })
            .collect();
        Self {
            semantic: ChannelCursor::new(semantic, semantic_high_cell_lineages),
            physical: ChannelCursor::new(physical, physical_high_cell_lineages),
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
        let sequence = tex_state::node_sequence::NodeSequence::mirrored(nodes);
        let semantic_high_cell_lineages = sequence.semantic_high_cell_lineages().to_vec();
        let physical_high_cell_lineages = sequence.physical_high_cell_lineages().to_vec();
        let (semantic, physical) = sequence.take();
        Self {
            physical: ChannelCursor::new(physical, physical_high_cell_lineages),
            semantic: ChannelCursor::new(semantic, semantic_high_cell_lineages),
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
        let mut high_cell_lineages = Vec::new();
        materialize_channel(
            state,
            &mut self.semantic,
            &decision,
            &self.params,
            Some(&self.actions),
            &mut line,
            &mut high_cell_lineages,
        );
        let mut physical_nodes = Vec::new();
        let mut physical_high_cell_lineages = Vec::new();
        materialize_channel(
            state,
            &mut self.physical,
            &self.physical_breaks[self.line_no],
            &self.params,
            None,
            &mut physical_nodes,
            &mut physical_high_cell_lineages,
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
            high_cell_lineages,
            physical_high_cell_lineages,
            penalty_after,
            hyphenated: decision.hyphenated,
            dimensions,
        })
    }
}

impl ChannelCursor {
    fn new(nodes: Vec<Node>, high_cell_lineages: Vec<DirectHighCellLineages>) -> Self {
        let node_count = nodes.len();
        assert_eq!(node_count, high_cell_lineages.len());
        Self {
            nodes: nodes.into_iter(),
            high_cell_lineages: high_cell_lineages.into_iter(),
            position: 0,
            node_count,
            pending_post: Vec::new(),
            pending_post_high_cell_lineages: Vec::new(),
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
    lineages: &mut Vec<DirectHighCellLineage>,
) {
    let end = decision.position.min(cursor.node_count);
    let start = cursor.position.min(end);
    let required = end
        .checked_sub(start)
        .and_then(|len| len.checked_add(cursor.pending_post.len()))
        .and_then(|len| len.checked_add(2))
        .expect("materialized line capacity fits usize");
    line.clear();
    lineages.clear();
    line.reserve(required);
    if params.left_skip != GlueSpec::ZERO {
        line.push(Node::Glue {
            spec: params.left_skip,
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
    lineages.append(&mut cursor.pending_post_high_cell_lineages);
    let (pending_post, pending_post_lineages) = push_owned_line_segment(
        state,
        (
            &mut cursor.nodes,
            &mut cursor.high_cell_lineages,
            &mut cursor.position,
            cursor.node_count,
        ),
        end,
        decision,
        &params.empty_list,
        actions,
        (line, lineages),
    );
    cursor.pending_post = pending_post;
    cursor.pending_post_high_cell_lineages = pending_post_lineages;
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
        spec: params.right_skip,
        kind: GlueKind::RightSkip,
        leader: None,
    });
    while cursor.nodes.as_slice().first().is_some_and(is_discardable) {
        let _ = cursor.nodes.next();
        let _ = cursor.high_cell_lineages.next();
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
    source: (
        &mut std::vec::IntoIter<Node>,
        &mut std::vec::IntoIter<DirectHighCellLineages>,
        &mut usize,
        usize,
    ),
    end: usize,
    decision: &BreakDecision,
    empty_list: &tex_state::node_arena::PageListId,
    actions: Option<&[MaterializationAction]>,
    output: (&mut Vec<Node>, &mut Vec<DirectHighCellLineage>),
) -> (Vec<Node>, Vec<DirectHighCellLineage>) {
    let (nodes, lineage_rows, position, node_count) = source;
    let (out, out_lineages) = output;
    let mut post = Vec::new();
    let mut post_lineages = Vec::new();
    while *position < end {
        let absolute = *position;
        let node = nodes.next().expect("paragraph break position is in bounds");
        let node_lineages = lineage_rows
            .next()
            .expect("paragraph lineage position is in bounds");
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
                    pre: *empty_list,
                    post: *empty_list,
                    replace: *empty_list,
                    physical_replace_count: 0,
                });
                out.extend(state.page_nodes(pre).to_vec());
                out_lineages.extend(frozen_high_cell_lineages(state, &pre, FrozenListRole::Pre));
                post.extend(state.page_nodes(post_list).to_vec());
                post_lineages.extend(frozen_high_cell_lineages(
                    state,
                    &post_list,
                    FrozenListRole::Post,
                ));
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
                out.extend(state.page_nodes(replace).to_vec());
                out_lineages.extend(frozen_high_cell_lineages(
                    state,
                    &replace,
                    FrozenListRole::Replace,
                ));
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
            node => {
                out.push(node);
                out_lineages.extend(node_lineages);
            }
        }
    }
    (post, post_lineages)
}

fn frozen_high_cell_lineages<S: TypesetState>(
    state: &S,
    list: &tex_state::node_arena::PageListId,
    role: FrozenListRole,
) -> Vec<DirectHighCellLineage> {
    state
        .page_nodes(*list)
        .iter()
        .enumerate()
        .flat_map(|(row, node)| {
            let count = match node {
                Node::Char { .. } => 1,
                Node::Lig { orig, .. } => orig.len(),
                _ => 0,
            };
            (0..count).map(move |unit| DirectHighCellLineage::Frozen {
                list: *list,
                row: u32::try_from(row).expect("frozen node list exceeds u32 rows"),
                unit: u32::try_from(unit).expect("ligature source exceeds u32 cells"),
                role,
            })
        })
        .collect()
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
