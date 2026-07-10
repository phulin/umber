use tex_state::glue::GlueSpec;
use tex_state::node::{GlueKind, Node};

use crate::TypesetState;

use super::{BreakDecision, BrokenLine, PostLineBreakParams};

pub fn post_line_break<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    breaks: &[BreakDecision],
    params: PostLineBreakParams,
) -> Vec<BrokenLine> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut pending_post = Vec::new();
    for (line_no, decision) in breaks.iter().enumerate() {
        let mut line = Vec::new();
        let dimensions = params.shape.dimensions(line_no + 1);
        if state.glue(params.left_skip) != GlueSpec::ZERO {
            line.push(Node::Glue {
                spec: params.left_skip,
                kind: GlueKind::LeftSkip,
                leader: None,
            });
        }
        line.append(&mut pending_post);
        let post = push_line_segment(state, nodes, start, decision, &mut line);
        pending_post = post;
        line.push(Node::Glue {
            spec: params.right_skip,
            kind: GlueKind::RightSkip,
            leader: None,
        });

        let penalty_after = line_penalty_after(line_no, breaks, decision.hyphenated, &params);
        lines.push(BrokenLine {
            nodes: line,
            penalty_after,
            hyphenated: decision.hyphenated,
            dimensions,
        });
        start = next_start(nodes, decision.position);
    }
    lines
}

fn push_line_segment<S: TypesetState>(
    state: &S,
    nodes: &[Node],
    start: usize,
    decision: &BreakDecision,
    out: &mut Vec<Node>,
) -> Vec<Node> {
    let end = decision.position.min(nodes.len());
    let mut post = Vec::new();
    for (offset, node) in nodes[start..end].iter().enumerate() {
        let absolute = start + offset;
        match node {
            Node::Disc {
                pre,
                post: post_list,
                ..
            } if decision.hyphenated && absolute + 1 == end => {
                out.extend_from_slice(state.nodes(*pre));
                post.extend_from_slice(state.nodes(*post_list));
            }
            Node::Disc { replace, .. } => out.extend_from_slice(state.nodes(*replace)),
            Node::Glue { .. } if absolute + 1 == end && end < nodes.len() => {}
            Node::MathOff(_) if absolute + 1 == end && end < nodes.len() => {
                out.push(Node::MathOff(tex_state::scaled::Scaled::from_raw(0)));
            }
            _ => out.push(node.clone()),
        }
    }
    post
}

fn next_start(nodes: &[Node], position: usize) -> usize {
    let mut start = position.min(nodes.len());
    while start < nodes.len() && is_discardable(&nodes[start]) {
        start += 1;
    }
    start
}

fn line_penalty_after(
    line_no: usize,
    breaks: &[BreakDecision],
    hyphenated: bool,
    params: &PostLineBreakParams,
) -> Option<i32> {
    if line_no + 1 >= breaks.len() {
        return None;
    }
    let mut penalty = params.interline_penalty;
    if line_no == 0 {
        penalty = penalty.saturating_add(params.club_penalty);
    }
    if line_no + 2 == breaks.len() {
        penalty = penalty.saturating_add(params.widow_penalty);
    }
    if hyphenated {
        penalty = penalty.saturating_add(params.broken_penalty);
    }
    (penalty != 0).then_some(penalty)
}

fn is_discardable(node: &Node) -> bool {
    matches!(
        node,
        Node::Glue { .. }
            | Node::Kern { .. }
            | Node::Penalty(_)
            | Node::MathOn(_)
            | Node::MathOff(_)
    )
}
