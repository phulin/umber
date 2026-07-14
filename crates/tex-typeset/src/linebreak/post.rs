use tex_state::glue::GlueSpec;
use tex_state::node::{GlueKind, KernKind, Node};

use crate::TypesetState;

use super::{BreakDecision, BrokenLine, PostLineBreakParams};

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
    nodes: std::iter::Peekable<std::iter::Enumerate<std::vec::IntoIter<Node>>>,
    node_count: usize,
    breaks: Vec<BreakDecision>,
    line_no: usize,
    pending_post: Vec<Node>,
    params: PostLineBreakParams,
}

impl LineMaterializer {
    #[must_use]
    pub fn new(nodes: Vec<Node>, breaks: Vec<BreakDecision>, params: PostLineBreakParams) -> Self {
        let node_count = nodes.len();
        Self {
            nodes: nodes.into_iter().enumerate().peekable(),
            node_count,
            breaks,
            line_no: 0,
            pending_post: Vec::new(),
            params,
        }
    }

    pub fn materialize_next<S: TypesetState>(
        &mut self,
        state: &S,
        mut line: Vec<Node>,
    ) -> Option<BrokenLine> {
        let decision = *self.breaks.get(self.line_no)?;
        let end = decision.position.min(self.node_count);
        let start = self.nodes.peek().map_or(end, |(index, _)| *index);
        let required = end
            .saturating_sub(start)
            .saturating_add(self.pending_post.len())
            .saturating_add(2);
        line.clear();
        line.reserve(required);

        let dimensions = self.params.shape.dimensions(self.line_no + 1);
        if state.glue(self.params.left_skip) != GlueSpec::ZERO {
            line.push(Node::Glue {
                spec: self.params.left_skip,
                kind: GlueKind::LeftSkip,
                leader: None,
            });
        }
        line.append(&mut self.pending_post);
        self.pending_post = push_owned_line_segment(
            state,
            &mut self.nodes,
            end,
            self.node_count,
            &decision,
            self.params.empty_list,
            &mut line,
        );
        line.push(Node::Glue {
            spec: self.params.right_skip,
            kind: GlueKind::RightSkip,
            leader: None,
        });

        let penalty_after = line_penalty_after(
            self.line_no,
            &self.breaks,
            decision.hyphenated,
            &self.params,
        );
        self.line_no += 1;
        while matches!(self.nodes.peek(), Some((_, node)) if is_discardable(node)) {
            let _ = self.nodes.next();
        }
        Some(BrokenLine {
            nodes: line,
            penalty_after,
            hyphenated: decision.hyphenated,
            dimensions,
        })
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
    let mut materializer = LineMaterializer::new(nodes, breaks.to_vec(), params);
    while let Some(line) = materializer.materialize_next(state, Vec::new()) {
        lines.push(line);
    }
    lines
}

fn push_owned_line_segment<S: TypesetState>(
    state: &S,
    nodes: &mut std::iter::Peekable<impl Iterator<Item = (usize, Node)>>,
    end: usize,
    node_count: usize,
    decision: &BreakDecision,
    empty_list: tex_state::ids::NodeListId,
    out: &mut Vec<Node>,
) -> Vec<Node> {
    let mut post = Vec::new();
    while matches!(nodes.peek(), Some((index, _)) if *index < end) {
        let (absolute, node) = nodes.next().expect("peeked paragraph node exists");
        match node {
            Node::Disc {
                pre,
                post: post_list,
                ..
            } if decision.hyphenated && absolute + 1 == end => {
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
            } => {
                out.push(Node::Disc {
                    kind,
                    pre,
                    post,
                    replace: empty_list,
                });
                out.extend(state.nodes(replace).into_iter().map(|node| node.to_owned()));
            }
            Node::Glue { .. } if absolute + 1 == end && end < node_count => {}
            Node::MathOff(_) if absolute + 1 == end && end < node_count => {
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
    penalty = penalty.saturating_add(
        penalty_array_value(&params.club_penalties, line_no + 1).unwrap_or(if line_no == 0 {
            params.club_penalty
        } else {
            0
        }),
    );
    let lines_from_end = breaks.len() - line_no - 1;
    penalty = penalty.saturating_add(
        penalty_array_value(&params.widow_penalties, lines_from_end).unwrap_or(
            if line_no + 2 == breaks.len() {
                params.widow_penalty
            } else {
                0
            },
        ),
    );
    if hyphenated {
        penalty = penalty.saturating_add(params.broken_penalty);
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
